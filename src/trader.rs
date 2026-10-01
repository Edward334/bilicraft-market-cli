use crate::{
    config::RiskConfig,
    db::Database,
    domain::{AiDecision, PortfolioSnapshot, StockCompany, TradeAction, TradeRequest},
    risk,
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};

pub struct DecisionExecutor<'a> {
    pub risk: &'a RiskConfig,
    pub db: &'a Database,
}
impl<'a> DecisionExecutor<'a> {
    pub fn validate_ai_decision(
        &self,
        decision: &AiDecision,
        company: &StockCompany,
        portfolio: &PortfolioSnapshot,
        daily_trades: u32,
        daily_loss: f64,
        last_trade_at: Option<DateTime<Utc>>,
    ) -> Result<TradeRequest> {
        let action = decision.action;
        match action {
            TradeAction::Hold | TradeAction::NoTrade => {
                return Err(anyhow!("decision does not request a trade"));
            }
            TradeAction::Buy | TradeAction::Sell => {}
        }
        let id = decision
            .company_id
            .ok_or_else(|| anyhow!("AI decision missing company_id"))?;
        if id != company.id {
            return Err(anyhow!("decision company does not match loaded company"));
        }
        let shares = decision
            .shares
            .ok_or_else(|| anyhow!("AI decision missing shares"))?;
        let req = TradeRequest {
            action,
            company_id: id,
            shares,
            source: "ai".into(),
            reason: Some(decision.reason.clone()),
        };
        risk::validate(risk::RiskContext {
            config: self.risk,
            now: Utc::now(),
            portfolio,
            company,
            request: &req,
            daily_trades,
            daily_loss,
            last_trade_at,
        })?;
        self.db.audit(
            "ai_decision",
            "ai",
            "approved",
            &serde_json::to_string(decision)?,
        )?;
        Ok(req)
    }
    pub fn reject(&self, source: &str, message: &str) -> Result<()> {
        self.db.audit("risk", source, "rejected", message)
    }
    pub fn record_result(&self, source: &str, status: &str, message: &str) -> Result<()> {
        self.db.audit("trade", source, status, message)
    }
}
pub fn decision_fingerprint(decision: &AiDecision, at: DateTime<Utc>) -> String {
    let v = serde_json::json!({"decision":decision,"minute":at.timestamp()/60});
    format!("{:x}", sha2::Sha256::digest(v.to_string().as_bytes()))
}
use sha2::Digest;
