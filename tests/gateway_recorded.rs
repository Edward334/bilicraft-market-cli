use anyhow::{Result, anyhow};
use bilicraft_ai_stock::{
    domain::TradeAction,
    gateway::{CommandFuture, CommandSender, StockCommandGateway},
    minecraft::ChatEvent,
};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
struct RecordedCommandSender {
    responses: Arc<HashMap<String, Vec<ChatEvent>>>,
    connected: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl RecordedCommandSender {
    fn from_fixture() -> Self {
        #[derive(serde::Deserialize)]
        struct Fixture {
            command: String,
            events: Vec<ChatEvent>,
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/bilicraft_chat_responses.json");
        let fixtures: Vec<Fixture> =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let responses = fixtures
            .into_iter()
            .map(|fixture| (fixture.command, fixture.events))
            .collect();
        Self {
            responses: Arc::new(responses),
            connected: Arc::new(AtomicBool::new(true)),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }
}

impl CommandSender for RecordedCommandSender {
    fn execute<'a>(&'a self, command: String) -> CommandFuture<'a> {
        let connected = self.connected.clone();
        let responses = self.responses.clone();
        let seen = self.seen.clone();
        Box::pin(async move {
            seen.lock().unwrap().push(command.clone());
            if !connected.load(Ordering::Acquire) {
                return Err(anyhow!("Minecraft disconnected: command timed out"));
            }
            responses
                .get(&command)
                .cloned()
                .ok_or_else(|| anyhow!("recorded response missing for {command}"))
        })
    }
}

#[tokio::test]
async fn recorded_bilicraft_commands_cover_balance_portfolio_company_and_trades() -> Result<()> {
    let sender = RecordedCommandSender::from_fixture();
    let gateway = StockCommandGateway::new(Arc::new(sender.clone()));

    assert_eq!(gateway.balance().await?, 1234.50);
    let holdings = gateway.portfolio().await?;
    assert_eq!(holdings.len(), 1);
    assert_eq!(holdings[0].company_name, "星环能源");
    assert_eq!(holdings[0].shares, 120);
    assert_eq!(holdings[0].total_value, 4567.8);

    let company = gateway.company_info(12).await?;
    assert_eq!(company.id, 12);
    assert_eq!(company.name, "星环能源");
    assert_eq!(company.price, 32140.0);
    assert_eq!(company.status.as_deref(), Some("交易中"));
    assert_eq!(company.available_shares, Some(800));

    assert!(gateway.trade("buy", 12, 10).await?.contains("买入成功"));
    assert!(gateway.trade("sell", 12, 4).await?.contains("卖出成功"));
    assert!(gateway.trade("buy", 12, 999999).await.is_err());

    assert_eq!(
        &*sender.seen.lock().unwrap(),
        &vec![
            "/bal".to_string(),
            "/invest portfolio".to_string(),
            "/invest company info 12".to_string(),
            "/invest buy 12 10".to_string(),
            "/invest sell 12 4".to_string(),
            "/invest buy 12 999999".to_string(),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn recorded_timeout_and_disconnect_recovery_are_explicit() -> Result<()> {
    let sender = RecordedCommandSender::from_fixture();
    let gateway = StockCommandGateway::new(Arc::new(sender.clone()));
    sender.set_connected(false);
    let timeout = gateway.balance().await.unwrap_err().to_string();
    assert!(timeout.contains("timed out"));

    sender.set_connected(true);
    assert_eq!(gateway.balance().await?, 1234.50);
    Ok(())
}

#[test]
fn recorded_fixture_keeps_trade_action_vocabulary_stable() {
    assert_eq!(serde_json::to_string(&TradeAction::Buy).unwrap(), "\"buy\"");
}
