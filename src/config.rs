use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub market: MarketConfig,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub risk: RiskConfig,
    #[serde(default)]
    pub qq: QqConfig,
    #[serde(default)]
    pub web: WebConfig,
    #[serde(default)]
    pub paths: PathsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub protocol: Option<i32>,
    #[serde(default)]
    pub player_name: Option<String>,
    #[serde(default)]
    pub auto_connect: bool,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 25565,
            protocol: Some(774),
            player_name: None,
            auto_connect: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConfig {
    pub base_url: String,
    #[serde(default = "default_interval")]
    pub interval: String,
    #[serde(default = "default_sync_seconds")]
    pub sync_seconds: u64,
}
fn default_interval() -> String {
    "15m".into()
}
fn default_sync_seconds() -> u64 {
    60
}
impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            base_url: "https://www.pleasance.icu".into(),
            interval: default_interval(),
            sync_seconds: default_sync_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub enabled: bool,
    pub base_url: String,
    #[serde(default = "default_chat_path")]
    pub chat_path: String,
    pub model: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_retries")]
    pub retries: u8,
    #[serde(default = "default_automation_seconds")]
    pub automation_interval_seconds: u64,
    #[serde(default)]
    pub system_prompt: Option<String>,
}
fn default_chat_path() -> String {
    "/v1/chat/completions".into()
}
fn default_timeout_seconds() -> u64 {
    45
}
fn default_retries() -> u8 {
    2
}
fn default_automation_seconds() -> u64 {
    900
}
impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://api.openai.com".into(),
            chat_path: default_chat_path(),
            model: "gpt-4o-mini".into(),
            timeout_seconds: default_timeout_seconds(),
            retries: default_retries(),
            automation_interval_seconds: default_automation_seconds(),
            system_prompt: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    #[serde(default)]
    pub automation_enabled: bool,
    #[serde(default)]
    pub optional_limits_enabled: bool,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub trading_windows: Vec<TimeWindow>,
    #[serde(default)]
    pub automation_windows: Vec<TimeWindow>,
    #[serde(default)]
    pub ai_budget: f64,
    #[serde(default)]
    pub ai_budget_percent: f64,
    #[serde(default)]
    pub max_single_trade: f64,
    #[serde(default)]
    pub max_daily_loss: f64,
    #[serde(default)]
    pub max_daily_trades: u32,
    #[serde(default)]
    pub cooldown_seconds: u64,
    #[serde(default)]
    pub max_positions: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start: String,
    pub end: String,
}
impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            automation_enabled: false,
            optional_limits_enabled: false,
            timezone: "UTC".into(),
            trading_windows: vec![],
            automation_windows: vec![],
            ai_budget: 0.0,
            ai_budget_percent: 0.0,
            max_single_trade: 0.0,
            max_daily_loss: 0.0,
            max_daily_trades: 0,
            cooldown_seconds: 300,
            max_positions: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QqConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_base_url: String,
    #[serde(default)]
    pub targets: Vec<QqTarget>,
    #[serde(default)]
    pub markdown: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QqTarget {
    Channel { id: String },
    Group { openid: String },
    User { openid: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    pub bind: String,
    #[serde(default)]
    pub static_dir: Option<PathBuf>,
}
impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8787".into(),
            static_dir: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    pub data_dir: PathBuf,
    pub secrets_file: PathBuf,
    pub database_file: PathBuf,
}
impl Default for PathsConfig {
    fn default() -> Self {
        let data = PathBuf::from("data");
        Self {
            data_dir: data.clone(),
            secrets_file: data.join("secrets.json"),
            database_file: data.join("bilicraft.sqlite3"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Secrets {
    #[serde(default)]
    pub minecraft_player_name: Option<String>,
    #[serde(default)]
    pub microsoft_refresh_token: Option<String>,
    #[serde(default)]
    pub minecraft_access_token: Option<String>,
    #[serde(default)]
    pub minecraft_uuid: Option<String>,
    #[serde(default)]
    pub qq_app_id: Option<String>,
    #[serde(default)]
    pub qq_app_secret: Option<String>,
    #[serde(default)]
    pub ai_api_key: Option<String>,
    #[serde(default)]
    pub qq_access_token: Option<String>,
    #[serde(default)]
    pub qq_access_token_expires_at: Option<i64>,
    #[serde(default)]
    pub web_password_hash: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).context("read config")?;
        Ok(toml::from_str(&text).context("parse config")?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            server: Default::default(),
            market: Default::default(),
            ai: Default::default(),
            risk: Default::default(),
            qq: Default::default(),
            web: Default::default(),
            paths: Default::default(),
        }
    }
}

pub fn load_secrets(path: &Path) -> Result<Secrets> {
    if !path.exists() {
        return Ok(Secrets::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}
pub fn save_secrets(path: &Path, secrets: &Secrets) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(secrets)?)?;
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
