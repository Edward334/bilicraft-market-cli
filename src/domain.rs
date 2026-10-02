use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockCompany {
    pub id: i64,
    #[serde(default)]
    pub market_id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub risk_level: Option<i32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub latest_price: Option<f64>,
    #[serde(default)]
    pub change_pct: Option<f64>,
    #[serde(default)]
    pub available_shares: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KlinePoint {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(default)]
    pub volume: i64,
    #[serde(default)]
    pub time: Option<i64>,
    #[serde(default)]
    pub bucket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockHolding {
    pub company_name: String,
    pub shares: i64,
    pub total_value: f64,
    #[serde(default)]
    pub market_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioSnapshot {
    pub balance: f64,
    pub holdings: Vec<StockHolding>,
    pub total_assets: f64,
    pub tracked_cost: f64,
    pub unrealized_pnl: f64,
    pub realized_pnl: f64,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeAction {
    Buy,
    Sell,
    Hold,
    NoTrade,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiDecision {
    pub action: TradeAction,
    #[serde(default)]
    pub company_id: Option<i64>,
    #[serde(default)]
    pub shares: Option<i64>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub stop: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRequest {
    pub action: TradeAction,
    pub company_id: i64,
    pub shares: i64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub kind: String,
    pub source: String,
    pub status: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEvent {
    pub kind: String,
    pub message: String,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub state: String,
    pub minecraft_connected: bool,
    pub player_name: Option<String>,
    pub last_market_sync: Option<DateTime<Utc>>,
    pub ai_enabled: bool,
    pub automation_enabled: bool,
    pub circuit_breaker: bool,
    #[serde(default)]
    pub minecraft_error: Option<String>,
}
