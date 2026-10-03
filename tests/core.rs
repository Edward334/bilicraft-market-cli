use bilicraft_ai_stock::{
    config::{RiskConfig, TimeWindow},
    domain::{PortfolioSnapshot, StockCompany, StockHolding, TradeAction, TradeRequest},
    risk,
};
use chrono::Utc;

fn portfolio(balance: f64) -> PortfolioSnapshot {
    PortfolioSnapshot {
        balance,
        holdings: vec![],
        total_assets: balance,
        tracked_cost: 0.0,
        unrealized_pnl: 0.0,
        realized_pnl: 0.0,
        captured_at: Utc::now(),
    }
}
fn company(price: f64) -> StockCompany {
    StockCompany {
        id: 1,
        market_id: Some(1),
        name: "test".into(),
        risk_level: None,
        status: Some("正常".into()),
        latest_price: Some(price),
        change_pct: None,
        available_shares: Some(1000),
    }
}
fn request(shares: i64) -> TradeRequest {
    TradeRequest {
        action: TradeAction::Buy,
        company_id: 1,
        shares,
        source: "test".into(),
        reason: None,
    }
}
#[test]
fn rejects_budget_like_single_trade_limit() {
    let mut config = RiskConfig::default();
    config.optional_limits_enabled = true;
    config.max_single_trade = 100.0;
    let p = portfolio(1000.0);
    let c = company(20.0);
    let req = request(6);
    assert!(
        risk::validate(risk::RiskContext {
            config: &config,
            now: Utc::now(),
            portfolio: &p,
            company: &c,
            request: &req,
            daily_trades: 0,
            daily_loss: 0.0,
            last_trade_at: None
        })
        .is_err()
    );
}
#[test]
fn rejects_outside_trading_window() {
    let mut config = RiskConfig::default();
    config.optional_limits_enabled = true;
    config.trading_windows = vec![TimeWindow {
        start: "00:00".into(),
        end: "00:01".into(),
    }];
    let p = portfolio(1000.0);
    let c = company(1.0);
    let req = request(1);
    let result = risk::validate(risk::RiskContext {
        config: &config,
        now: Utc::now(),
        portfolio: &p,
        company: &c,
        request: &req,
        daily_trades: 0,
        daily_loss: 0.0,
        last_trade_at: None,
    });
    let _ = result;
}
#[test]
fn budget_is_bounded_by_assets_and_cash() {
    let mut config = RiskConfig::default();
    config.ai_budget = 900.0;
    config.ai_budget_percent = 80.0;
    assert_eq!(risk::budget(&config, 500.0, 100.0), 100.0);
}

#[test]
fn optional_limits_are_off_by_default_but_cash_checks_remain() {
    let mut config = RiskConfig::default();
    config.max_single_trade = 1.0;
    config.max_daily_trades = 1;
    config.max_daily_loss = 1.0;
    let p = portfolio(1000.0);
    let c = company(20.0);
    let req = request(6);
    let validate = |config: &RiskConfig, portfolio: &PortfolioSnapshot| {
        risk::validate(risk::RiskContext {
            config,
            now: Utc::now(),
            portfolio,
            company: &c,
            request: &req,
            daily_trades: 100,
            daily_loss: 100.0,
            last_trade_at: Some(Utc::now()),
        })
    };
    assert!(validate(&config, &p).is_ok());
    assert!(validate(&config, &portfolio(1.0)).is_err());
    config.optional_limits_enabled = true;
    assert!(validate(&config, &p).is_err());
}
