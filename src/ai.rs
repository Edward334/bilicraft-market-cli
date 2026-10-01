use crate::{
    config::AiConfig,
    domain::{AiDecision, PortfolioSnapshot, StockCompany},
};
use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: String,
}
#[derive(Debug, Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
    response_format: serde_json::Value,
}
#[derive(Debug, Deserialize)]
struct Response {
    choices: Vec<Choice>,
}
#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}
#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: String,
}
#[derive(Clone)]
pub struct AiProvider {
    client: Client,
    config: AiConfig,
    key: Option<String>,
}
impl AiProvider {
    pub fn new(client: Client, config: AiConfig, key: Option<String>) -> Self {
        Self {
            client,
            config,
            key,
        }
    }
    pub async fn decide(
        &self,
        companies: &[StockCompany],
        portfolio: &PortfolioSnapshot,
        question: Option<&str>,
    ) -> Result<(AiDecision, String)> {
        let key = self
            .key
            .as_deref()
            .context("AI API key is not configured")?;
        let context =
            serde_json::json!({"companies":companies,"portfolio":portfolio,"question":question})
                .to_string();
        let system=self.config.system_prompt.clone().unwrap_or_else(||"你是 Bilicraft 虚拟股市交易策略。只能返回 JSON，字段 action=buy|sell|hold|no_trade、company_id、shares、reason、confidence、stop。不得假设资金或持仓，执行器会重新校验。".into());
        let body = Request {
            model: &self.config.model,
            messages: vec![
                Message {
                    role: "system",
                    content: system,
                },
                Message {
                    role: "user",
                    content: context,
                },
            ],
            temperature: 0.2,
            response_format: serde_json::json!({"type":"json_object"}),
        };
        let url = format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            self.config.chat_path
        );
        let mut last = None;
        for attempt in 0..=self.config.retries {
            let r = self
                .client
                .post(&url)
                .bearer_auth(key)
                .json(&body)
                .timeout(std::time::Duration::from_secs(self.config.timeout_seconds))
                .send()
                .await;
            match r {
                Ok(resp) => match resp.error_for_status() {
                    Ok(v) => {
                        let x: Response = v.json().await?;
                        let content = x
                            .choices
                            .first()
                            .context("AI returned no choices")?
                            .message
                            .content
                            .clone();
                        let json = extract_json(&content)?;
                        let d: AiDecision =
                            serde_json::from_str(&json).context("invalid AI decision schema")?;
                        return Ok((d, content));
                    }
                    Err(e) => last = Some(e.to_string()),
                },
                Err(e) => last = Some(e.to_string()),
            }
            if attempt < self.config.retries {
                tokio::time::sleep(std::time::Duration::from_millis(400 * (attempt as u64 + 1)))
                    .await
            }
        }
        Err(anyhow!(
            "AI request failed: {}",
            last.unwrap_or_else(|| "unknown error".into())
        ))
    }
}
fn extract_json(s: &str) -> Result<String> {
    let s = s
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let a = s.find('{').context("AI output has no JSON object")?;
    let b = s.rfind('}').context("AI output has incomplete JSON")?;
    Ok(s[a..=b].to_owned())
}
