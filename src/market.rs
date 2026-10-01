use crate::{
    config::MarketConfig,
    domain::{KlinePoint, StockCompany},
};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Health {
    pub companies: i64,
    pub prices: i64,
    #[serde(default)]
    pub latest_price_at: Option<String>,
    #[serde(default)]
    pub latest_cash_at: Option<String>,
    #[serde(default)]
    pub last_update_marker: Option<String>,
}
#[derive(Debug, Clone)]
pub struct MarketClient {
    client: Client,
    config: MarketConfig,
}
impl MarketClient {
    pub fn new(client: Client, config: MarketConfig) -> Self {
        Self { client, config }
    }
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(self
            .client
            .get(format!(
                "{}{}",
                self.config.base_url.trim_end_matches('/'),
                path
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub async fn companies(&self) -> Result<Vec<StockCompany>> {
        self.get("/api/companies").await
    }
    pub async fn health(&self) -> Result<Health> {
        self.get("/api/health").await
    }
    pub async fn kline(&self, id: i64, interval: &str) -> Result<Vec<KlinePoint>> {
        let v: serde_json::Value = self
            .get(&format!("/api/kline/{id}?interval={interval}"))
            .await?;
        serde_json::from_value(v.get("data").cloned().context("missing kline data")?)
            .context("invalid kline")
    }
    pub async fn available_shares(
        &self,
        id: i64,
        interval: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let v: serde_json::Value = self
            .get(&format!("/api/available-shares/{id}?interval={interval}"))
            .await?;
        Ok(v.get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }
}
