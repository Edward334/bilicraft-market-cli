use crate::domain::{StockCompany, StockHolding};
use crate::minecraft::SessionHandle;
use crate::minecraft::{ChatEvent, normalize_chat};
use anyhow::{Result, anyhow};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::time::{sleep, timeout};

pub type CommandFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<ChatEvent>>> + Send + 'a>>;
pub trait CommandSender: Send + Sync {
    fn execute<'a>(&'a self, command: String) -> CommandFuture<'a>;
}

impl CommandSender for SessionHandle {
    fn execute<'a>(&'a self, command: String) -> CommandFuture<'a> {
        Box::pin(async move {
            let mut events = self.subscribe();
            self.send_command(command).await?;
            let mut collected = Vec::new();
            let mut quiet = false;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
            while tokio::time::Instant::now() < deadline {
                let wait = if collected.is_empty() {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(900)
                };
                match timeout(wait, events.recv()).await {
                    Ok(Ok(event)) => {
                        collected.push(event);
                        quiet = false;
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                    Ok(Err(_)) | Err(_) => {
                        quiet = true;
                        break;
                    }
                }
                if quiet {
                    break;
                }
            }
            if collected.is_empty() {
                return Err(anyhow!("Minecraft command timed out"));
            }
            sleep(Duration::from_millis(50)).await;
            Ok(collected)
        })
    }
}

#[derive(Clone)]
pub struct StockCommandGateway<S> {
    sender: Arc<S>,
}
impl<S: CommandSender + 'static> StockCommandGateway<S> {
    pub fn new(sender: Arc<S>) -> Self {
        Self { sender }
    }
    pub async fn balance(&self) -> Result<f64> {
        let lines = self.sender.execute("/bal".into()).await?;
        parse_balance(&lines)
    }
    pub async fn portfolio(&self) -> Result<Vec<StockHolding>> {
        let lines = self.sender.execute("/invest portfolio".into()).await?;
        parse_portfolio(&lines)
    }
    pub async fn company_info(&self, id: i64) -> Result<LiveStockInfo> {
        let lines = self
            .sender
            .execute(format!("/invest company info {id}"))
            .await?;
        parse_company_info(&lines, id)
    }
    pub async fn trade(&self, action: &str, id: i64, shares: i64) -> Result<String> {
        if !matches!(action, "buy" | "sell") {
            return Err(anyhow!("invalid trade action"));
        }
        let lines = self
            .sender
            .execute(format!("/invest {action} {id} {shares}"))
            .await?;
        let failure = lines
            .iter()
            .map(|x| normalize_chat(&x.text))
            .find(|x| looks_failure(x));
        if let Some(x) = failure {
            return Err(anyhow!(x));
        }
        lines
            .iter()
            .map(|x| normalize_chat(&x.text))
            .find(|x| looks_trade_success(x, action))
            .ok_or_else(|| anyhow!("server did not confirm trade"))
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LiveStockInfo {
    pub id: i64,
    pub name: String,
    pub price: f64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub available_shares: Option<i64>,
}
fn text(lines: &[ChatEvent]) -> String {
    lines
        .iter()
        .map(|x| normalize_chat(&x.text))
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn parse_balance(lines: &[ChatEvent]) -> Result<f64> {
    let t = text(lines);
    let re = regex::Regex::new(r"(?:资金|余额)[:：]?\s*\$?([0-9,]+(?:\.[0-9]+)?)").unwrap();
    let n = re
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|v| v.as_str().replace(',', "").parse().ok())
        .ok_or_else(|| anyhow!("could not parse balance"))?;
    Ok(n)
}
pub fn parse_company_info(lines: &[ChatEvent], requested: i64) -> Result<LiveStockInfo> {
    let t = text(lines);
    let name = lines
        .iter()
        .map(|x| normalize_chat(&x.text))
        .map(|x| x.trim().to_string())
        .find(|x| {
            !x.is_empty()
                && !x.contains("帕拉伦股市")
                && !x.starts_with("Id:")
                && !x.starts_with("状态:")
                && !x.starts_with("价格:")
                && !x.starts_with("风险等级:")
                && !x.starts_with("可用股数:")
                && !x.contains("=-=-=-=-")
        })
        .ok_or_else(|| anyhow!("company name missing"))?;
    let id = regex::Regex::new(r"(?i)Id[:：]\s*(\d+)")
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|v| v.as_str().parse().ok())
        .unwrap_or(requested);
    let price = parse_price(&t).ok_or_else(|| anyhow!("company price missing"))?;
    let status = regex::Regex::new(r"状态[:：]\s*([^\n]+)")
        .unwrap()
        .captures(&t)
        .map(|c| c[1].trim().to_owned());
    let available = regex::Regex::new(r"可用股数[:：]\s*(\d+)")
        .unwrap()
        .captures(&t)
        .and_then(|c| c[1].parse().ok());
    Ok(LiveStockInfo {
        id,
        name,
        price,
        status,
        available_shares: available,
    })
}
pub fn parse_price(t: &str) -> Option<f64> {
    let c = regex::Regex::new(r"价格[:：]\s*([0-9,]+(?:\.[0-9]+)?)\s*([KMBT]?)")
        .unwrap()
        .captures(t)?;
    let n: f64 = c[1].replace(',', "").parse().ok()?;
    let m = match &c[2].to_uppercase()[..] {
        "K" => 1e3,
        "M" => 1e6,
        "B" => 1e9,
        "T" => 1e12,
        _ => 1.0,
    };
    Some(n * m)
}
pub fn parse_portfolio(lines: &[ChatEvent]) -> Result<Vec<StockHolding>> {
    let mut out = Vec::new();
    let mut name = None;
    let mut id = None;
    let mut shares = None;
    for l in lines
        .iter()
        .map(|x| normalize_chat(&x.text))
        .map(|x| x.trim().to_owned())
        .filter(|x| !x.is_empty())
    {
        if l.starts_with("股票:") || l.starts_with("股票：") {
            shares = regex::Regex::new(r"股票[:：]\s*(\d+)")
                .unwrap()
                .captures(&l)
                .and_then(|c| c[1].parse().ok())
        } else if l.starts_with("总价值:") || l.starts_with("总价值：") {
            let value = regex::Regex::new(r"总价值[:：]\s*([0-9]+(?:\.[0-9]+)?)")
                .unwrap()
                .captures(&l)
                .and_then(|c| c[1].parse().ok())
                .ok_or_else(|| anyhow!("invalid portfolio value"))?;
            if let (Some(n), Some(s)) = (name.take(), shares.take()) {
                out.push(StockHolding {
                    company_name: n,
                    shares: s,
                    total_value: value,
                    market_id: id.take(),
                })
            }
        } else if !l.contains("帕拉伦股市") && !l.contains("=-=-=-=-") && !l.starts_with('[') {
            let indexed =
                regex::Regex::new(r"^(?:(?:#|Id[:：]?\s*)?(-?\d+)\s*[·.、:：-]\s*)?(.+)$")
                    .unwrap()
                    .captures(&l);
            if let Some(captures) = indexed {
                id = captures
                    .get(1)
                    .and_then(|value| value.as_str().parse().ok());
                name = captures
                    .get(2)
                    .map(|value| value.as_str().trim().to_owned());
            } else {
                id = None;
                name = Some(l.clone());
            }
        }
    }
    Ok(out)
}
fn looks_failure(s: &str) -> bool {
    [
        "失败",
        "错误",
        "不足",
        "无法",
        "不存在",
        "无效",
        "禁止",
        "没有",
        "超出",
    ]
    .iter()
    .any(|x| s.contains(x))
}
fn looks_trade_success(s: &str, a: &str) -> bool {
    let compact = s.replace(' ', "");
    let words = if a == "buy" {
        ["买入", "购买"].as_slice()
    } else {
        ["卖出", "出售", "售出"].as_slice()
    };
    words.iter().any(|x| compact.contains(x))
        && ["成功", "已买入", "已购买", "已卖出", "已出售", "已售出"]
            .iter()
            .any(|x| compact.contains(x))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(s: &str) -> ChatEvent {
        ChatEvent {
            text: s.into(),
            raw: s.into(),
            timestamp: 0,
        }
    }
    #[test]
    fn parses_balance() {
        assert_eq!(parse_balance(&[e("余额：1,234.50 元")]).unwrap(), 1234.5)
    }
    #[test]
    fn parses_price_units() {
        assert_eq!(parse_price("价格: 1.2M"), Some(1_200_000.0))
    }
    #[test]
    fn parses_portfolio_zero_server_id() {
        let holdings = parse_portfolio(&[
            e("帕拉伦股市"),
            e("0 · 帕拉伦联合储蓄"),
            e("股票: 1"),
            e("总价值: 61.84"),
        ])
        .unwrap();
        assert_eq!(holdings[0].market_id, Some(0));
        assert_eq!(holdings[0].shares, 1);
    }
    #[test]
    fn parses_trade_failure() {
        assert!(looks_failure("资金不足"))
    }
}
