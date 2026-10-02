use crate::{
    ai::AiProvider,
    app::AppState,
    config::{Secrets, save_secrets},
    domain::{AppEvent, PortfolioSnapshot, StockHolding, TradeAction, TradeRequest},
    gateway::StockCommandGateway,
    ledger::PurchaseRecord,
    minecraft::{self, ConnectionState, SessionHandle},
    risk,
};
use anyhow::anyhow;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response, Sse},
    routing::{get, post, put},
};
use chrono::Utc;
use futures_util::Stream;
use serde::Deserialize;
use std::sync::atomic::Ordering;
use std::{convert::Infallible, net::SocketAddr, time::Duration};
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let bind = state.config.web.bind.parse::<SocketAddr>()?;
    let app = Router::new()
        .route("/api/status", get(status))
        .route("/api/market/companies", get(companies))
        .route("/api/market/health", get(health))
        .route("/api/market/kline/{id}", get(kline))
        .route("/api/market/available-shares/{id}", get(available_shares))
        .route("/api/watchlist", get(watchlist).post(set_watchlist))
        .route("/api/portfolio", get(portfolio))
        .route("/api/portfolio/refresh", post(portfolio))
        .route("/api/portfolio/history", get(portfolio_history))
        .route("/api/audit", get(audit))
        .route("/api/events", get(events))
        .route("/api/trade", post(trade))
        .route("/api/ai/run", post(ai_run))
        .route("/api/ai/pause", post(ai_pause))
        .route("/api/ai/resume", post(ai_resume))
        .route("/api/auth/status", get(auth_status))
        .route("/api/config", get(config).put(update_config))
        .route("/api/secrets", put(update_secrets))
        .route("/api/auth/device", post(auth_device))
        .route("/api/qq/status", get(qq_status))
        .route("/api/qq/setup", post(qq_setup))
        .route("/api/qq/test", post(qq_test))
        .route("/api/connect", post(connect))
        .route("/api/disconnect", post(disconnect))
        .route("/", get(index));
    crate::scheduler::spawn(state.clone());
    if state.config.qq.enabled {
        let qq = state.qq.clone();
        tokio::spawn(async move {
            if let Err(error) = crate::qq::run_default_gateway(&qq).await {
                tracing::error!(%error, "QQ Gateway stopped");
            }
        });
    }
    let app = app.with_state(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("Web UI: http://{bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Html(include_str!("../web/dist/index.html").to_string()),
    )
}
async fn status(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(s.status()).unwrap())
}
async fn auth_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let sec = s.secrets.read();
    Json(
        serde_json::json!({"state": if sec.minecraft_access_token.is_some() {"authenticated"} else {"signed_out"}, "player_name": sec.minecraft_player_name, "uuid": sec.minecraft_uuid}),
    )
}
async fn companies(
    State(s): State<AppState>,
) -> Result<Json<Vec<crate::domain::StockCompany>>, ApiError> {
    s.market.companies().await.map(Json).map_err(ApiError::from)
}
async fn health(State(s): State<AppState>) -> Result<Json<crate::market::Health>, ApiError> {
    s.market.health().await.map(Json).map_err(ApiError::from)
}
async fn kline(
    Path(id): Path<i64>,
    State(s): State<AppState>,
) -> Result<Json<Vec<crate::domain::KlinePoint>>, ApiError> {
    let config = s.config_snapshot();
    s.market
        .kline(id, &config.market.interval)
        .await
        .map(Json)
        .map_err(ApiError::from)
}
async fn available_shares(
    Path(id): Path<i64>,
    State(s): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let config = s.config_snapshot();
    s.market
        .available_shares(id, &config.market.interval)
        .await
        .map(Json)
        .map_err(ApiError::from)
}
async fn watchlist(State(s): State<AppState>) -> Result<Json<Vec<i64>>, ApiError> {
    s.db.lock().watchlist().map(Json).map_err(ApiError::from)
}
async fn set_watchlist(
    State(s): State<AppState>,
    Json(value): Json<serde_json::Value>,
) -> Result<Json<Vec<i64>>, ApiError> {
    let id = value
        .get("company_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::bad("company_id missing"))?;
    let enabled = value
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    s.db.lock().set_watch(id, enabled).map_err(ApiError::from)?;
    s.db.lock().watchlist().map(Json).map_err(ApiError::from)
}

async fn portfolio(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let session = current_session(&s)?;
    let gateway = StockCommandGateway::new(std::sync::Arc::new(session));
    let balance = gateway.balance().await.map_err(ApiError::from)?;
    let holdings = enrich_holding_ids(
        gateway.portfolio().await.map_err(ApiError::from)?,
        &s.market.companies().await.map_err(ApiError::from)?,
    );
    let metrics =
        s.db.lock()
            .holding_metrics(&holdings)
            .map_err(ApiError::from)?;
    let market_value: f64 = holdings.iter().map(|h| h.total_value).sum();
    let tracked_cost: f64 = metrics
        .iter()
        .filter_map(|v| v.get("cost").and_then(|x| x.as_f64()))
        .sum();
    let unrealized_pnl: f64 = metrics
        .iter()
        .filter_map(|v| v.get("pnl").and_then(|x| x.as_f64()))
        .sum();
    let realized_pnl = s.db.lock().realized_pnl().map_err(ApiError::from)?;
    let snapshot = PortfolioSnapshot {
        balance,
        holdings: holdings.clone(),
        total_assets: balance + market_value,
        tracked_cost,
        unrealized_pnl,
        realized_pnl,
        captured_at: Utc::now(),
    };
    s.db.lock()
        .save_snapshot(&snapshot)
        .map_err(ApiError::from)?;
    s.db.lock()
        .record_profit_snapshot(&snapshot)
        .map_err(ApiError::from)?;
    Ok(Json(
        serde_json::json!({"balance":balance,"holdings":metrics,"total_assets":snapshot.total_assets,"tracked_cost":tracked_cost,"unrealized_pnl":unrealized_pnl,"realized_pnl":realized_pnl,"captured_at":snapshot.captured_at}),
    ))
}
async fn portfolio_history(
    State(s): State<AppState>,
) -> Result<Json<Vec<crate::ledger::ProfitRecord>>, ApiError> {
    s.db.lock()
        .profit_history(365)
        .map(Json)
        .map_err(ApiError::from)
}
async fn audit(
    State(s): State<AppState>,
) -> Result<Json<Vec<crate::domain::AuditEntry>>, ApiError> {
    s.db.lock().audits(100).map(Json).map_err(ApiError::from)
}

async fn trade(
    State(s): State<AppState>,
    Json(req): Json<TradeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let config = s.config_snapshot();
    let session = current_session(&s)?;
    let gateway = StockCommandGateway::new(std::sync::Arc::new(session));
    let balance = gateway.balance().await.map_err(ApiError::from)?;
    let companies = s.market.companies().await.map_err(ApiError::from)?;
    let holdings = enrich_holding_ids(
        gateway.portfolio().await.map_err(ApiError::from)?,
        &companies,
    );
    let company = companies
        .into_iter()
        .find(|c| c.id == req.company_id || c.market_id == Some(req.company_id))
        .ok_or_else(|| ApiError::bad("company not found"))?;
    let server_company_id = company.market_id.unwrap_or(company.id);
    let live = gateway
        .company_info(server_company_id)
        .await
        .map_err(ApiError::from)?;
    let mut company = company;
    company.latest_price = Some(live.price);
    company.status = live.status;
    company.available_shares = live.available_shares;
    let effective_request = TradeRequest {
        company_id: server_company_id,
        ..req.clone()
    };
    let metrics =
        s.db.lock()
            .holding_metrics(&holdings)
            .map_err(ApiError::from)?;
    let total_assets = balance + holdings.iter().map(|h| h.total_value).sum::<f64>();
    let portfolio = PortfolioSnapshot {
        balance,
        holdings,
        total_assets,
        tracked_cost: metrics
            .iter()
            .filter_map(|v| v.get("cost").and_then(|x| x.as_f64()))
            .sum(),
        unrealized_pnl: metrics
            .iter()
            .filter_map(|v| v.get("pnl").and_then(|x| x.as_f64()))
            .sum(),
        realized_pnl: s.db.lock().realized_pnl().map_err(ApiError::from)?,
        captured_at: Utc::now(),
    };
    let (daily_trades, daily_loss, last_trade_at) = {
        let db = s.db.lock();
        (
            db.daily_trade_count().map_err(ApiError::from)?,
            db.daily_loss().map_err(ApiError::from)?,
            db.last_successful_trade_at().map_err(ApiError::from)?,
        )
    };
    risk::validate(risk::RiskContext {
        config: &config.risk,
        now: Utc::now(),
        portfolio: &portfolio,
        company: &company,
        request: &effective_request,
        daily_trades,
        daily_loss,
        last_trade_at,
    })
    .map_err(|e| {
        let _ =
            s.db.lock()
                .audit("risk", &req.source, "rejected", &e.to_string());
        ApiError::from(e)
    })?;
    let action = match req.action {
        TradeAction::Buy => "buy",
        TradeAction::Sell => "sell",
        _ => return Err(ApiError::bad("manual trade action must be buy or sell")),
    };
    let result = gateway.trade(action, server_company_id, req.shares).await;
    match result {
        Ok(message) => {
            if action == "buy" {
                s.db.lock()
                    .record_buy(
                        server_company_id,
                        &company.name,
                        req.shares,
                        company.latest_price.unwrap_or(0.0),
                    )
                    .map_err(ApiError::from)?;
            } else {
                s.db.lock()
                    .record_sell(server_company_id, req.shares, live.price, false)
                    .map_err(ApiError::from)?;
            }
            s.db.lock()
                .audit("trade", &req.source, "success", &message)
                .map_err(ApiError::from)?;
            s.emit("trade",&message,serde_json::json!({"company_id":server_company_id,"market_api_id":company.id,"action":action,"shares":req.shares}));
            Ok(Json(
                serde_json::json!({"status":"confirmed","message":message}),
            ))
        }
        Err(e) => {
            let _ =
                s.db.lock()
                    .audit("trade", &req.source, "failed", &e.to_string());
            Err(ApiError::from(e))
        }
    }
}

#[derive(Deserialize, Default)]
struct AiRunRequest {
    question: Option<String>,
}
async fn ai_run(
    State(s): State<AppState>,
    Json(input): Json<AiRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let config = s.config_snapshot();
    if !s.ai_enabled.load(Ordering::Relaxed) {
        return Err(ApiError::bad("AI is disabled"));
    }
    if s.circuit_breaker.load(Ordering::Relaxed) {
        return Err(ApiError::bad("AI circuit breaker is active"));
    }
    let session = current_session(&s)?;
    let gateway = StockCommandGateway::new(std::sync::Arc::new(session));
    let balance = gateway.balance().await.map_err(ApiError::from)?;
    let companies = s.market.companies().await.map_err(ApiError::from)?;
    let holdings = enrich_holding_ids(
        gateway.portfolio().await.map_err(ApiError::from)?,
        &companies,
    );
    let metrics =
        s.db.lock()
            .holding_metrics(&holdings)
            .map_err(ApiError::from)?;
    let portfolio = PortfolioSnapshot {
        balance,
        holdings,
        total_assets: balance
            + metrics
                .iter()
                .filter_map(|v| v.get("total_value").and_then(|x| x.as_f64()))
                .sum::<f64>(),
        tracked_cost: metrics
            .iter()
            .filter_map(|v| v.get("cost").and_then(|x| x.as_f64()))
            .sum(),
        unrealized_pnl: metrics
            .iter()
            .filter_map(|v| v.get("pnl").and_then(|x| x.as_f64()))
            .sum(),
        realized_pnl: s.db.lock().realized_pnl().map_err(ApiError::from)?,
        captured_at: Utc::now(),
    };
    let key = s.secrets.read().ai_api_key.clone();
    let provider = AiProvider::new(s.client.clone(), config.ai.clone(), key);
    let (decision, raw) = provider
        .decide(&companies, &portfolio, input.question.as_deref())
        .await
        .map_err(ApiError::from)?;
    s.db.lock()
        .audit("ai_decision", "ai", "received", &raw)
        .map_err(ApiError::from)?;
    if matches!(decision.action, TradeAction::Hold | TradeAction::NoTrade) {
        return Ok(Json(
            serde_json::json!({"status":"no_trade","decision":decision}),
        ));
    }
    let id = decision
        .company_id
        .ok_or_else(|| ApiError::bad("AI decision missing company_id"))?;
    let company = companies
        .into_iter()
        .find(|c| c.id == id || c.market_id == Some(id))
        .ok_or_else(|| ApiError::bad("AI company not found"))?;
    let server_company_id = company.market_id.unwrap_or(company.id);
    let live = gateway
        .company_info(server_company_id)
        .await
        .map_err(ApiError::from)?;
    let mut company = company;
    company.latest_price = Some(live.price);
    company.status = live.status;
    company.available_shares = live.available_shares;
    let shares = decision
        .shares
        .ok_or_else(|| ApiError::bad("AI decision missing shares"))?;
    let amount = company.latest_price.unwrap_or(0.0) * shares as f64;
    let budget = risk::budget(&config.risk, portfolio.total_assets, portfolio.balance);
    if decision.action == TradeAction::Buy && amount > budget {
        let message = format!("AI budget exceeded: amount={amount:.2}, budget={budget:.2}");
        let _ = s.db.lock().audit("risk", "ai", "rejected", &message);
        return Err(ApiError::bad(message));
    }
    let request = TradeRequest {
        action: decision.action,
        company_id: server_company_id,
        shares,
        source: "ai".into(),
        reason: Some(decision.reason.clone()),
    };
    let (daily_trades, daily_loss, last_trade_at) = {
        let db = s.db.lock();
        (
            db.daily_trade_count().map_err(ApiError::from)?,
            db.daily_loss().map_err(ApiError::from)?,
            db.last_successful_trade_at().map_err(ApiError::from)?,
        )
    };
    risk::validate(risk::RiskContext {
        config: &config.risk,
        now: Utc::now(),
        portfolio: &portfolio,
        company: &company,
        request: &request,
        daily_trades,
        daily_loss,
        last_trade_at,
    })
    .map_err(|e| {
        let _ = s.db.lock().audit("risk", "ai", "rejected", &e.to_string());
        ApiError::from(e)
    })?;
    let action = if decision.action == TradeAction::Buy {
        "buy"
    } else {
        "sell"
    };
    let message = gateway
        .trade(action, server_company_id, shares)
        .await
        .map_err(ApiError::from)?;
    if action == "buy" {
        s.db.lock()
            .record_buy(
                server_company_id,
                &company.name,
                shares,
                company.latest_price.unwrap_or(0.0),
            )
            .map_err(ApiError::from)?
    } else {
        s.db.lock()
            .record_sell(server_company_id, shares, live.price, false)
            .map_err(ApiError::from)?
    };
    s.db.lock()
        .audit("trade", "ai", "success", &message)
        .map_err(ApiError::from)?;
    if config.risk.max_daily_loss > 0.0
        && s.db.lock().daily_loss().unwrap_or(0.0) >= config.risk.max_daily_loss
    {
        s.circuit_breaker.store(true, Ordering::Relaxed);
        s.automation_enabled.store(false, Ordering::Relaxed);
        s.emit(
            "risk",
            "AI circuit breaker activated by daily loss limit",
            serde_json::json!({}),
        );
    }
    s.emit("ai_trade",&message,serde_json::json!({"company_id":server_company_id,"market_api_id":company.id,"action":action,"shares":shares,"reason":decision.reason}));
    Ok(Json(
        serde_json::json!({"status":"confirmed","decision":decision,"message":message}),
    ))
}
async fn ai_pause(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut c = s.config_snapshot();
    c.risk.automation_enabled = false;
    s.automation_enabled.store(false, Ordering::Relaxed);
    c.save(&s.config_file).map_err(ApiError::from)?;
    *s.runtime_config.write() = c;
    Ok(Json(serde_json::json!({"automation_enabled":false})))
}
async fn ai_resume(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    if !s.ai_enabled.load(Ordering::Relaxed) {
        return Err(ApiError::bad("AI is disabled"));
    }
    let mut c = s.config_snapshot();
    c.risk.automation_enabled = true;
    s.circuit_breaker.store(false, Ordering::Relaxed);
    s.automation_enabled.store(true, Ordering::Relaxed);
    c.save(&s.config_file).map_err(ApiError::from)?;
    *s.runtime_config.write() = c;
    Ok(Json(serde_json::json!({"automation_enabled":true})))
}

async fn config(State(s): State<AppState>) -> Json<crate::config::Config> {
    Json(s.config_snapshot())
}
async fn update_config(
    State(s): State<AppState>,
    Json(patch): Json<serde_json::Value>,
) -> Result<Json<crate::config::Config>, ApiError> {
    let mut merged =
        serde_json::to_value(s.config_snapshot()).map_err(|e| ApiError::bad(e.to_string()))?;
    merge_json(&mut merged, patch);
    let next: crate::config::Config =
        serde_json::from_value(merged).map_err(|e| ApiError::bad(e.to_string()))?;
    next.save(&s.config_file).map_err(ApiError::from)?;
    *s.runtime_config.write() = next.clone();
    s.ai_enabled.store(next.ai.enabled, Ordering::Relaxed);
    s.automation_enabled
        .store(next.risk.automation_enabled, Ordering::Relaxed);
    Ok(Json(next))
}
fn merge_json(target: &mut serde_json::Value, patch: serde_json::Value) {
    if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            if value.is_object() && target.get(key).map(|v| v.is_object()).unwrap_or(false) {
                merge_json(target.get_mut(key).expect("key exists"), value.clone());
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}
#[derive(Deserialize)]
struct SecretUpdate {
    ai_api_key: Option<String>,
    qq_app_id: Option<String>,
    qq_app_secret: Option<String>,
}
async fn update_secrets(
    State(s): State<AppState>,
    Json(input): Json<SecretUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut next = s.secrets.read().clone();
    if let Some(value) = input.ai_api_key {
        next.ai_api_key = (!value.trim().is_empty()).then_some(value);
    }
    if let Some(value) = input.qq_app_id {
        next.qq_app_id = (!value.trim().is_empty()).then_some(value);
    }
    if let Some(value) = input.qq_app_secret {
        next.qq_app_secret = (!value.trim().is_empty()).then_some(value);
    }
    let secrets_path = s.config_snapshot().paths.secrets_file;
    save_secrets(&secrets_path, &next).map_err(ApiError::from)?;
    *s.secrets.write() = next;
    Ok(Json(serde_json::json!({"saved":true})))
}
async fn auth_device(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let info = crate::auth::request_device_code(&s.client)
        .await
        .map_err(ApiError::from)?;
    let response = serde_json::json!({"verification_uri":info.verification_uri,"user_code":info.user_code,"expires_in":info.expires_in});
    *s.auth_device.write() = Some(info.clone());
    let state = s.clone();
    tokio::spawn(async move {
        let mut secrets = state.secrets.read().clone();
        match crate::auth::complete_device_login(
            &state.client,
            &info,
            &state.config_snapshot().paths.secrets_file,
            &mut secrets,
        )
        .await
        {
            Ok(session) => {
                *state.secrets.write() = secrets;
                state.emit(
                    "auth",
                    "Microsoft login completed",
                    serde_json::json!({"player_name":session.name,"uuid":session.uuid}),
                );
                if state.config_snapshot().server.auto_connect {
                    if let Err(error) = connect(State(state.clone())).await {
                        let _ =
                            state
                                .db
                                .lock()
                                .audit("minecraft", "web", "failed", &error.to_string());
                        state.emit(
                            "error",
                            "Minecraft auto-connect failed",
                            serde_json::json!({}),
                        );
                    }
                }
            }
            Err(error) => {
                let _ = state
                    .db
                    .lock()
                    .audit("auth", "web", "failed", &error.to_string());
                state.emit("error", "Microsoft login failed", serde_json::json!({}));
            }
        }
        *state.auth_device.write() = None;
    });
    Ok(Json(response))
}
async fn qq_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let sec = s.secrets.read();
    let binding = s.qq.binding_state();
    Json(
        serde_json::json!({"configured":sec.qq_app_id.is_some()&&sec.qq_app_secret.is_some(),"state":if binding.qr_url.is_some(){"waiting_for_scan"}else{"ready"},"qr_url":binding.qr_url,"qr_expired":binding.qr_expired,"targets":s.qq.targets()}),
    )
}
async fn qq_setup(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let service = s.qq.clone();
    tokio::spawn(async move {
        if let Err(error) = service.setup_qr().await {
            tracing::error!(%error,"QQ QR setup failed");
        }
    });
    Ok(Json(serde_json::json!({"status":"waiting_for_scan"})))
}
async fn qq_test(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    s.qq.send_text("Bilicraft AI Stock QQ Bot connection test")
        .await
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({"status":"sent"})))
}

async fn connect(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let sec = s.secrets.read().clone();
    let name = s
        .config_snapshot()
        .server
        .player_name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or(sec
            .minecraft_player_name
            .filter(|value| !value.trim().is_empty()))
        .ok_or_else(|| ApiError::bad("player_name is not configured"))?;
    let uuid = sec
        .minecraft_uuid
        .ok_or_else(|| ApiError::bad("Minecraft UUID is not configured; run login"))?;
    let token = sec.minecraft_access_token.unwrap_or_default();
    let config = s.config_snapshot();
    let protocol = config.server.protocol.unwrap_or(0);
    let session = minecraft::connect(
        config.server.host.clone(),
        config.server.port,
        protocol,
        name,
        uuid,
        token,
    )
    .await
    .map_err(ApiError::from)?;
    *s.minecraft.write() = Some(session.clone());
    *s.minecraft_state.write() = ConnectionState::LoggingIn;
    let state = s.minecraft_state.clone();
    let error_state = s.minecraft_error.clone();
    *error_state.write() = None;
    let events = s.events.clone();
    let mut rx = session.state.clone();
    let mut error_rx = session.error.clone();
    *s.minecraft_state.write() = *rx.borrow();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() { break; }
                    let now = *rx.borrow();
                    *state.write() = now;
                    if matches!(now, ConnectionState::Disconnected | ConnectionState::Failed) {
                        *error_state.write() = Some(format!("Minecraft session ended in {now:?}"));
                    }
                    let _ = events.send(AppEvent {
                        kind: "minecraft".into(),
                        message: format!("Minecraft connection state: {now:?}"),
                        at: Utc::now(),
                        data: serde_json::json!({"state":format!("{now:?}")}),
                    });
                    if matches!(now, ConnectionState::Disconnected | ConnectionState::Failed) { break; }
                }
                changed = error_rx.changed() => {
                    if changed.is_err() { break; }
                    if let Some(error) = error_rx.borrow().clone() { *error_state.write() = Some(error); }
                }
            }
        }
    });
    Ok(Json(serde_json::json!({"status":"connecting"})))
}
async fn disconnect(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let session = { s.minecraft.write().take() };
    if let Some(session) = session {
        session.disconnect().await.map_err(ApiError::from)?;
    }
    *s.minecraft_state.write() = ConnectionState::Disconnected;
    Ok(Json(serde_json::json!({"status":"disconnected"})))
}
fn enrich_holding_ids(
    holdings: Vec<StockHolding>,
    companies: &[crate::domain::StockCompany],
) -> Vec<StockHolding> {
    holdings
        .into_iter()
        .map(|holding| {
            if holding.market_id.is_some() {
                return holding;
            }
            let market_id = companies
                .iter()
                .find(|company| company.name == holding.company_name)
                .and_then(|company| company.market_id.or(Some(company.id)));
            StockHolding {
                market_id,
                ..holding
            }
        })
        .collect()
}

fn current_session(s: &AppState) -> Result<SessionHandle, ApiError> {
    s.minecraft
        .read()
        .clone()
        .ok_or_else(|| ApiError::bad("Minecraft is not connected"))
}

async fn events(
    State(s): State<AppState>,
) -> Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
    let stream = BroadcastStream::new(s.events.subscribe())
        .filter_map(|x| x.ok())
        .map(|e: AppEvent| Ok(axum::response::sse::Event::default().json_data(e).unwrap()));
    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
}
#[derive(Debug)]
struct ApiError(String);
impl ApiError {
    fn bad(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ApiError {}
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self(e.to_string())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":self.0})),
        )
            .into_response()
    }
}
