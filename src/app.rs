use crate::{
    config::{Config, Secrets, load_secrets},
    db::Database,
    domain::{AppEvent, ServiceStatus},
    market::MarketClient,
    minecraft::{ConnectionState, SessionHandle},
    qq::QqService,
};
use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;
use std::{
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::sync::broadcast;
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub config_file: std::path::PathBuf,
    pub runtime_config: Arc<RwLock<Config>>,
    pub secrets: Arc<RwLock<Secrets>>,
    pub db: Arc<parking_lot::Mutex<Database>>,
    pub market: MarketClient,
    pub events: broadcast::Sender<AppEvent>,
    pub client: reqwest::Client,
    pub minecraft: Arc<parking_lot::RwLock<Option<SessionHandle>>>,
    pub minecraft_state: Arc<parking_lot::RwLock<ConnectionState>>,
    pub minecraft_error: Arc<parking_lot::RwLock<Option<String>>>,
    pub ai_enabled: Arc<AtomicBool>,
    pub automation_enabled: Arc<AtomicBool>,
    pub circuit_breaker: Arc<AtomicBool>,
    pub automation_running: Arc<AtomicBool>,
    pub trade_lock: Arc<tokio::sync::Mutex<()>>,
    pub qq: QqService,
    pub auth_device: Arc<parking_lot::RwLock<Option<crate::auth::DeviceCodeInfo>>>,
    pub web_auth: crate::web_auth::WebAuthState,
}
impl AppState {
    pub fn new(
        config: Config,
        config_file: std::path::PathBuf,
        db: Database,
        secrets: Secrets,
        client: reqwest::Client,
    ) -> Self {
        let (market_tx, _) = broadcast::channel(256);
        let ai_enabled = config.ai.enabled;
        let automation_enabled = config.risk.automation_enabled;
        let secrets = Arc::new(RwLock::new(secrets));
        let qq = QqService::new(
            client.clone(),
            config.qq.clone(),
            secrets.clone(),
            config.paths.secrets_file.clone(),
        );
        Self {
            market: MarketClient::new(client.clone(), config.market.clone()),
            config_file,
            runtime_config: Arc::new(RwLock::new(config.clone())),
            config,
            secrets,
            db: Arc::new(parking_lot::Mutex::new(db)),
            events: market_tx,
            client,
            minecraft: Arc::new(parking_lot::RwLock::new(None)),
            minecraft_state: Arc::new(parking_lot::RwLock::new(ConnectionState::Disconnected)),
            minecraft_error: Arc::new(parking_lot::RwLock::new(None)),
            ai_enabled: Arc::new(AtomicBool::new(ai_enabled)),
            automation_enabled: Arc::new(AtomicBool::new(automation_enabled)),
            circuit_breaker: Arc::new(AtomicBool::new(false)),
            automation_running: Arc::new(AtomicBool::new(false)),
            trade_lock: Arc::new(tokio::sync::Mutex::new(())),
            qq,
            auth_device: Arc::new(parking_lot::RwLock::new(None)),
            web_auth: crate::web_auth::WebAuthState::new(),
        }
    }
    pub fn config_snapshot(&self) -> Config {
        self.runtime_config.read().clone()
    }
    pub fn emit(&self, kind: &str, message: &str, data: serde_json::Value) {
        let _ = self.events.send(AppEvent {
            kind: kind.into(),
            message: message.into(),
            at: Utc::now(),
            data,
        });
    }
    pub fn status(&self) -> ServiceStatus {
        ServiceStatus {
            state: "ready".into(),
            minecraft_connected: *self.minecraft_state.read() == ConnectionState::Connected,
            player_name: self.secrets.read().minecraft_player_name.clone(),
            last_market_sync: None,
            ai_enabled: self.ai_enabled.load(Ordering::Relaxed),
            automation_enabled: self.automation_enabled.load(Ordering::Relaxed),
            circuit_breaker: self.circuit_breaker.load(Ordering::Relaxed),
            minecraft_error: self.minecraft_error.read().clone(),
        }
    }
    pub fn load(root: &std::path::Path) -> Result<Self> {
        let config_path = root.join("config.toml");
        let mut c = Config::load(&config_path)?;
        c.paths.data_dir = root.join(&c.paths.data_dir);
        c.paths.secrets_file = root.join(&c.paths.secrets_file);
        c.paths.database_file = root.join(&c.paths.database_file);
        let s = load_secrets(&c.paths.secrets_file)?;
        let db = Database::open(&c.paths.database_file)?;
        db.init_ledger()?;
        Ok(Self::new(
            c,
            config_path,
            db,
            s,
            reqwest::Client::builder()
                .user_agent(crate::APP_NAME)
                .build()?,
        ))
    }
}
