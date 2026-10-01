use crate::{
    config::{RiskConfig, TimeWindow},
    domain::{PortfolioSnapshot, StockCompany, TradeAction, TradeRequest},
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Timelike, Utc};
#[derive(Debug, Clone)]
pub struct RiskContext<'a> {
    pub config: &'a RiskConfig,
    pub now: DateTime<Utc>,
    pub portfolio: &'a PortfolioSnapshot,
    pub company: &'a StockCompany,
    pub request: &'a TradeRequest,
    pub daily_trades: u32,
    pub daily_loss: f64,
    pub last_trade_at: Option<DateTime<Utc>>,
}
pub fn validate(c: RiskContext<'_>) -> Result<()> {
    let r = c.config;
    if c.request.shares <= 0 {
        return Err(anyhow!("shares must be positive"));
    }
    if !in_windows(c.now, r.trading_windows.as_slice()) {
        return Err(anyhow!("outside trading window"));
    }
    if r.max_daily_trades > 0 && c.daily_trades >= r.max_daily_trades {
        return Err(anyhow!("daily trade limit reached"));
    }
    if r.max_daily_loss > 0.0 && c.daily_loss <= -r.max_daily_loss {
        return Err(anyhow!("daily loss limit reached"));
    }
    if let Some(last) = c.last_trade_at {
        if (c.now - last).num_seconds() < (r.cooldown_seconds as i64) {
            return Err(anyhow!("trade cooldown active"));
        }
    }
    if let Some(status) = &c.company.status {
        let bad = status.contains("破产")
            || status.contains("停牌")
            || status.to_lowercase().contains("bankrupt");
        if bad {
            return Err(anyhow!("company is not tradable: {status}"));
        }
    }
    if r.max_positions > 0
        && c.request.action == TradeAction::Buy
        && !c
            .portfolio
            .holdings
            .iter()
            .any(|h| h.market_id == Some(c.company.id))
        && c.portfolio.holdings.len() as u32 >= r.max_positions
    {
        return Err(anyhow!("maximum positions reached"));
    }
    let price = c.company.latest_price.unwrap_or(0.0);
    let amount = price * c.request.shares as f64;
    if r.max_single_trade > 0.0 && amount > r.max_single_trade {
        return Err(anyhow!("single trade limit exceeded"));
    }
    if c.request.action == TradeAction::Buy {
        if amount > c.portfolio.balance {
            return Err(anyhow!("insufficient available balance"));
        }
        if let Some(available) = c.company.available_shares {
            if c.request.shares > available {
                return Err(anyhow!("requested shares exceed available shares"));
            }
        }
    }
    if c.request.action == TradeAction::Sell {
        let held = c
            .portfolio
            .holdings
            .iter()
            .find(|h| h.market_id == Some(c.company.id))
            .map(|h| h.shares)
            .unwrap_or(0);
        if c.request.shares > held {
            return Err(anyhow!("requested shares exceed current holding"));
        }
    }
    Ok(())
}
pub fn budget(r: &RiskConfig, total_assets: f64, cash: f64) -> f64 {
    let fixed = if r.ai_budget > 0.0 {
        r.ai_budget
    } else {
        f64::INFINITY
    };
    let pct = if r.ai_budget_percent > 0.0 {
        total_assets * r.ai_budget_percent / 100.0
    } else {
        f64::INFINITY
    };
    fixed.min(pct).min(total_assets.max(0.0)).min(cash.max(0.0))
}
pub fn in_windows(now: DateTime<Utc>, windows: &[TimeWindow]) -> bool {
    if windows.is_empty() {
        return true;
    }
    let mins = now.hour() * 60 + now.minute();
    windows.iter().any(|w| {
        let p = |s: &str| -> Option<u32> {
            let mut x = s.split(':');
            Some(
                x.next()?
                    .parse::<u32>()
                    .ok()?
                    .checked_mul(60)?
                    .checked_add(x.next()?.parse().ok()?)?,
            )
        };
        let (a, b) = (p(&w.start), p(&w.end));
        match (a, b) {
            (Some(a), Some(b)) => {
                if a <= b {
                    mins >= a && mins <= b
                } else {
                    mins >= a || mins <= b
                }
            }
            _ => false,
        }
    })
}
