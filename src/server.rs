use crate::{
    ai::AiProvider,
    app::AppState,
    config::{Secrets, save_secrets},
    domain::{AppEvent, PortfolioSnapshot, StockHolding, TradeAction, TradeRequest},
    gateway::StockCommandGateway,
    ledger::PurchaseRecord,
    minecraft::{self, ConnectionState, SessionHandle},
    risk,
    web_auth::{AuthState, WebAuthSecrets, WebAuthState},
};
use anyhow::anyhow;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    middleware::Next,
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
        .route("/api/ai/capital", get(ai_capital))
        .route("/api/ai/pause", post(ai_pause))
        .route("/api/ai/resume", post(ai_resume))
        .route("/api/auth/status", get(auth_status))
        .route("/api/web-auth/status", get(web_auth_status))
        .route("/api/web-auth/setup", post(web_auth_setup))
        .route("/api/web-auth/login", post(web_auth_login))
        .route("/api/web-auth/logout", post(web_auth_logout))
        .route("/api/web-auth/password", put(web_auth_password))
        .route("/api/config", get(config).put(update_config))
        .route("/api/secrets", put(update_secrets))
        .route("/api/auth/device", post(auth_device))
        .route("/api/qq/status", get(qq_status))
        .route("/api/qq/setup", post(qq_setup))
        .route("/api/qq/test", post(qq_test))
        .route("/api/connect", post(connect))
        .route("/api/disconnect", post(disconnect))
        .route("/", get(index))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web_auth_middleware,
        ));
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

fn web_auth_secrets(secrets: &crate::config::Secrets) -> WebAuthSecrets {
    WebAuthSecrets {
        password_hash: secrets.web_password_hash.clone(),
    }
}

fn web_auth_state(s: &AppState, headers: &axum::http::HeaderMap) -> AuthState {
    s.web_auth
        .state(&web_auth_secrets(&s.secrets.read()), headers)
}

async fn web_auth_middleware(
    State(s): State<AppState>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let public = path == "/"
        || matches!(
            path,
            "/api/web-auth/status"
                | "/api/web-auth/setup"
                | "/api/web-auth/login"
                | "/api/web-auth/logout"
        );
    if public || !path.starts_with("/api/") {
        return next.run(request).await;
    }
    if matches!(
        web_auth_state(&s, request.headers()),
        AuthState::Authenticated
    ) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"web authentication required"})),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
struct WebPasswordInput {
    password: String,
    confirmation: String,
}

async fn web_auth_status(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Json<serde_json::Value> {
    let state = web_auth_state(&s, &headers);
    Json(serde_json::json!({
        "state": match state { AuthState::SetupRequired => "setup_required", AuthState::LoginRequired => "login_required", AuthState::Authenticated => "authenticated" },
        "password_configured": !matches!(state, AuthState::SetupRequired),
    }))
}

async fn web_auth_setup(
    State(s): State<AppState>,
    Json(input): Json<WebPasswordInput>,
) -> Result<impl IntoResponse, ApiError> {
    let mut secrets = s.secrets.read().clone();
    let mut auth_secrets = web_auth_secrets(&secrets);
    s.web_auth
        .set_password(
            &mut auth_secrets,
            &input.password,
            &input.confirmation,
            false,
        )
        .map_err(ApiError::from)?;
    secrets.web_password_hash = auth_secrets.password_hash;
    save_secrets(&s.config_snapshot().paths.secrets_file, &secrets).map_err(ApiError::from)?;
    *s.secrets.write() = secrets;
    let token = s.web_auth.create_session();
    Ok((
        [(header::SET_COOKIE, WebAuthState::session_cookie(&token))],
        Json(serde_json::json!({"state":"authenticated"})),
    ))
}

async fn web_auth_login(
    State(s): State<AppState>,
    Json(input): Json<WebPasswordInput>,
) -> Result<impl IntoResponse, ApiError> {
    let secrets = s.secrets.read().clone();
    if !s
        .web_auth
        .verify_password(&web_auth_secrets(&secrets), &input.password)
    {
        return Err(ApiError::bad("invalid web password"));
    }
    let token = s.web_auth.create_session();
    Ok((
        [(header::SET_COOKIE, WebAuthState::session_cookie(&token))],
        Json(serde_json::json!({"state":"authenticated"})),
    ))
}

async fn web_auth_logout(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    s.web_auth.clear_session(&headers);
    (
        [(header::SET_COOKIE, WebAuthState::clear_cookie())],
        Json(serde_json::json!({"state":"logged_out"})),
    )
}

async fn web_auth_password(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(input): Json<WebPasswordInput>,
) -> Result<impl IntoResponse, ApiError> {
    if !matches!(web_auth_state(&s, &headers), AuthState::Authenticated) {
        return Err(ApiError::bad("web authentication required"));
    }
    let mut secrets = s.secrets.read().clone();
    let mut auth_secrets = web_auth_secrets(&secrets);
    s.web_auth
        .set_password(
            &mut auth_secrets,
            &input.password,
            &input.confirmation,
            true,
        )
        .map_err(ApiError::from)?;
    secrets.web_password_hash = auth_secrets.password_hash;
    save_secrets(&s.config_snapshot().paths.secrets_file, &secrets).map_err(ApiError::from)?;
    *s.secrets.write() = secrets;
    s.web_auth.clear_session(&headers);
    Ok((
        [(header::SET_COOKIE, WebAuthState::clear_cookie())],
        Json(serde_json::json!({"state":"password_changed","relogin_required":true})),
    ))
}
async fn auth_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let sec = s.secrets.read();
    let device = s.auth_device.read();
    let error = s.auth_error.read();
    let state = if device.is_some() {
        "waiting_for_login"
    } else if s.auth_requesting.load(Ordering::Relaxed) {
        "requesting_code"
    } else if error.is_some() {
        "failed"
    } else if sec.minecraft_access_token.is_some() {
        "authenticated"
    } else {
        "signed_out"
    };
    Json(
        serde_json::json!({"state":state, "error":*error, "player_name":sec.minecraft_player_name, "uuid":sec.minecraft_uuid,
        "user_code":device.as_ref().map(|d|&d.user_code), "verification_uri":device.as_ref().map(|d|&d.verification_uri)}),
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
    let _guard = s.trade_lock.lock().await;
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
        source: "manual".into(),
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
                    .record_owned_buy(
                        false,
                        server_company_id,
                        &company.name,
                        req.shares,
                        company.latest_price.unwrap_or(0.0),
                    )
                    .map_err(ApiError::from)?;
            } else {
                s.db.lock()
                    .record_owned_sell(false, server_company_id, req.shares, live.price)
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
    execute_ai(s, input.question, false).await
}

pub(crate) async fn run_automation(s: AppState) -> anyhow::Result<()> {
    execute_ai(s, None, true)
        .await
        .map(|_| ())
        .map_err(Into::into)
}

async fn ai_capital(State(s): State<AppState>) -> Result<Json<crate::ledger::AiCapital>, ApiError> {
    let config = s.config_snapshot();
    let session = s.minecraft.read().clone();
    let cash = if let Some(session) = session {
        Some(
            StockCommandGateway::new(std::sync::Arc::new(session))
                .balance()
                .await
                .map_err(ApiError::from)?,
        )
    } else {
        None
    };
    Ok(Json(
        s.db.lock()
            .ai_capital(config.risk.ai_budget, cash)
            .map_err(ApiError::from)?,
    ))
}

async fn execute_ai(
    s: AppState,
    question: Option<String>,
    automated: bool,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _guard = s.trade_lock.lock().await;
    if automated && !s.automation_enabled.load(Ordering::Relaxed) {
        return Err(ApiError::bad("AI automation is stopped"));
    }
    let config = s.config_snapshot();
    if !s.ai_enabled.load(Ordering::Relaxed) {
        return Err(ApiError::bad("AI is disabled"));
    }
    if config.risk.optional_limits_enabled && s.circuit_breaker.load(Ordering::Relaxed) {
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
    let capital =
        s.db.lock()
            .ai_capital(config.risk.ai_budget, Some(balance))
            .map_err(ApiError::from)?;
    if capital.principal <= 0.0 {
        return Err(ApiError::bad("Set AI principal before trading"));
    }
    let positions = s.db.lock().ai_positions().map_err(ApiError::from)?;
    let key = s.secrets.read().ai_api_key.clone();
    let provider = AiProvider::new(s.client.clone(), config.ai.clone(), key);
    let (decision, raw) = provider
        .decide(&companies, &portfolio, question.as_deref(), &serde_json::json!({"capital":capital,"positions":positions,"optional_limits":config.risk}))
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
    // Re-read spendable cash after the provider request; account balances may have changed.
    let cash = gateway.balance().await.map_err(ApiError::from)?;
    let config = s.config_snapshot();
    let capital =
        s.db.lock()
            .ai_capital(config.risk.ai_budget, Some(cash))
            .map_err(ApiError::from)?;
    let amount = company.latest_price.unwrap_or(0.0) * shares as f64;
    let budget = if config.risk.optional_limits_enabled && config.risk.ai_budget_percent > 0.0 {
        capital
            .available_cash
            .min(portfolio.total_assets * config.risk.ai_budget_percent / 100.0)
    } else {
        capital.available_cash
    };
    if decision.action == TradeAction::Sell
        && shares
            > positions
                .iter()
                .find(|p| p.company_id == server_company_id)
                .map(|p| p.shares)
                .unwrap_or(0)
    {
        return Err(ApiError::bad(
            "AI cannot sell shares outside its own positions",
        ));
    }
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
    if !s.ai_enabled.load(Ordering::Relaxed)
        || (automated && !s.automation_enabled.load(Ordering::Relaxed))
    {
        return Err(ApiError::bad(
            "AI automation stopped before order submission",
        ));
    }
    let message = gateway
        .trade(action, server_company_id, shares)
        .await
        .map_err(ApiError::from)?;
    if action == "buy" {
        s.db.lock()
            .record_owned_buy(
                true,
                server_company_id,
                &company.name,
                shares,
                company.latest_price.unwrap_or(0.0),
            )
            .map_err(ApiError::from)?
    } else {
        s.db.lock()
            .record_owned_sell(true, server_company_id, shares, live.price)
            .map_err(ApiError::from)?
    };
    s.db.lock()
        .audit("trade", "ai", "success", &message)
        .map_err(ApiError::from)?;
    if config.risk.optional_limits_enabled
        && config.risk.max_daily_loss > 0.0
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
    c.save(&s.config_file).map_err(ApiError::from)?;
    s.automation_enabled.store(false, Ordering::Relaxed);
    *s.runtime_config.write() = c;
    Ok(Json(serde_json::json!({"automation_enabled":false})))
}
async fn ai_resume(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut c = s.config_snapshot();
    if s.secrets
        .read()
        .ai_api_key
        .as_ref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(ApiError::bad("Configure AI API key first"));
    }
    if c.risk.ai_budget <= 0.0 {
        return Err(ApiError::bad("Set AI principal first"));
    }
    current_session(&s)?;
    c.ai.enabled = true;
    c.risk.automation_enabled = true;
    c.save(&s.config_file).map_err(ApiError::from)?;
    *s.runtime_config.write() = c;
    s.circuit_breaker.store(false, Ordering::Relaxed);
    s.ai_enabled.store(true, Ordering::Relaxed);
    s.automation_enabled.store(true, Ordering::Relaxed);
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
    validate_config_patch(&merged, &patch)?;
    merge_json(&mut merged, patch);
    let next: crate::config::Config =
        serde_json::from_value(merged).map_err(|e| ApiError::bad(e.to_string()))?;
    if !next.risk.ai_budget.is_finite() || next.risk.ai_budget < 0.0 {
        return Err(ApiError::bad(
            "AI principal must be finite and non-negative",
        ));
    }
    next.save(&s.config_file).map_err(ApiError::from)?;
    *s.runtime_config.write() = next.clone();
    s.ai_enabled.store(next.ai.enabled, Ordering::Relaxed);
    s.automation_enabled
        .store(next.risk.automation_enabled, Ordering::Relaxed);
    Ok(Json(next))
}
fn validate_config_patch(
    target: &serde_json::Value,
    patch: &serde_json::Value,
) -> Result<(), ApiError> {
    let fields = patch
        .as_object()
        .ok_or_else(|| ApiError::bad("configuration patch must be an object"))?;
    for (key, value) in fields {
        let existing = target
            .get(key)
            .ok_or_else(|| ApiError::bad(format!("unknown configuration field: {key}")))?;
        if existing.is_object() {
            validate_config_patch(existing, value)?;
        }
    }
    Ok(())
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
struct AuthRequestGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for AuthRequestGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

async fn auth_device(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    if s.auth_requesting.swap(true, Ordering::Relaxed) {
        return Err(ApiError::bad(
            "Microsoft login is already in progress; refresh login status",
        ));
    }
    let guard = AuthRequestGuard(s.auth_requesting.clone());
    *s.auth_error.write() = None;
    let info = crate::auth::request_device_code(&s.client)
        .await
        .map_err(|error| {
            let message = format!("{error:#}");
            *s.auth_error.write() = Some(message.clone());
            ApiError::bad(message)
        })?;
    let response = serde_json::json!({"verification_uri":info.verification_uri,"user_code":info.user_code,"expires_in":info.expires_in});
    *s.auth_device.write() = Some(info.clone());
    let state = s.clone();
    tokio::spawn(async move {
        let _guard = guard;
        let mut secrets = state.secrets.read().clone();
        let login = tokio::time::timeout(
            Duration::from_secs(info.expires_in + 60),
            crate::auth::complete_device_login_without_save(&state.client, &info, &mut secrets),
        )
        .await
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "Microsoft login timed out; request a new device code"
            ))
        });
        match login {
            Ok(session) => {
                // Preserve keys and Web password changes made while waiting for login.
                {
                    let mut current = state.secrets.write();
                    let mut next = current.clone();
                    next.minecraft_access_token = secrets.minecraft_access_token;
                    next.minecraft_player_name = secrets.minecraft_player_name;
                    next.minecraft_uuid = secrets.minecraft_uuid;
                    next.microsoft_refresh_token = secrets.microsoft_refresh_token;
                    if let Err(error) =
                        save_secrets(&state.config_snapshot().paths.secrets_file, &next)
                    {
                        *state.auth_error.write() =
                            Some(format!("Failed to save login: {error:#}"));
                        *state.auth_device.write() = None;
                        return;
                    }
                    *current = next;
                }
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
                *state.auth_error.write() = Some(format!("{error:#}"));
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

#[cfg(test)]
mod config_tests {
    use super::*;

    #[tokio::test]
    async fn auth_status_exposes_progress_and_errors_without_device_secret() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::load(root.path()).unwrap();
        state.auth_requesting.store(true, Ordering::Relaxed);
        assert_eq!(
            auth_status(State(state.clone())).await.0["state"],
            "requesting_code"
        );
        *state.auth_device.write() = Some(crate::auth::DeviceCodeInfo {
            device_code: "private-device-secret".into(),
            user_code: "TEST1234".into(),
            verification_uri: "https://www.microsoft.com/link".into(),
            interval: 5,
            expires_in: 900,
        });
        let status = auth_status(State(state.clone())).await.0;
        assert_eq!(status["state"], "waiting_for_login");
        assert_eq!(status["user_code"], "TEST1234");
        assert!(!status.to_string().contains("private-device-secret"));
        *state.auth_device.write() = None;
        state.auth_requesting.store(false, Ordering::Relaxed);
        *state.auth_error.write() = Some("Microsoft request timed out".into());
        let status = auth_status(State(state)).await.0;
        assert_eq!(status["state"], "failed");
        assert_eq!(status["error"], "Microsoft request timed out");
    }

    #[tokio::test]
    async fn stopped_automation_uses_shared_executor_without_http() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::load(root.path()).unwrap();
        let error = run_automation(state).await.unwrap_err().to_string();
        assert_eq!(error, "AI automation is stopped");
    }

    #[tokio::test]
    async fn config_save_persists_nested_fields_and_rejects_flat_payloads() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::load(root.path()).unwrap();
        let bad = update_config(
            State(state.clone()),
            Json(serde_json::json!({"ai_budget": 20000})),
        )
        .await;
        assert!(bad.is_err());
        assert!(!state.config_file.exists());
        let patch = serde_json::json!({"risk": {
            "ai_budget": 20000, "ai_budget_percent": 10,
            "max_single_trade": 5000, "max_daily_loss": 2000,
            "max_daily_trades": 5, "max_positions": 5,
            "automation_enabled": false, "trading_windows": [], "automation_windows": []
        }});
        let Json(saved) = update_config(State(state.clone()), Json(patch))
            .await
            .unwrap();
        assert_eq!(saved.risk.ai_budget, 20000.0);
        assert_eq!(state.config_snapshot().risk.max_positions, 5);
        let persisted = crate::config::Config::load(&state.config_file).unwrap();
        assert_eq!(persisted.risk.ai_budget_percent, 10.0);
        let restarted = AppState::load(root.path()).unwrap();
        assert_eq!(restarted.config_snapshot().risk.ai_budget, 20000.0);
        assert_eq!(restarted.config_snapshot().risk.max_single_trade, 5000.0);
        assert!(!restarted.status().automation_enabled);
    }
}
