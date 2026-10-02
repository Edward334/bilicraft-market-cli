//! Official QQ Bot integration.
//!
//! Sources:
//! - https://bot.q.qq.com/wiki/agent-qqbot/
//! - https://www.npmjs.com/package/@tencent-connect/qqbot-connector
//! - https://bot.q.qq.com/wiki/develop/api-v2/
//!
//! QR binding is deliberately a one-shot Node helper. The long-lived runtime is
//! Rust: it owns token refresh, HTTP messages, target authorization, and the
//! Gateway protocol state machine. `GatewayTransport` is kept as a small seam so
//! the application can provide its `tokio-tungstenite` transport after declaring
//! that crate in Cargo.toml.

use crate::config::{QqConfig, QqTarget, Secrets, save_secrets};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use futures_util::{Future, SinkExt, StreamExt};
use parking_lot::RwLock;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::{interval, sleep},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub const DEFAULT_API_BASE_URL: &str = "https://api.bot.qq.com";
pub const TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
pub const GATEWAY_URL_PATH: &str = "/gateway";
pub const MAX_MESSAGE_CHUNK_CHARS: usize = 3500;

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(deserialize_with = "deserialize_expiry_seconds")]
    expires_in: i64,
}

fn deserialize_expiry_seconds<'de, D>(deserializer: D) -> std::result::Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let seconds = match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|v| v as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    };
    seconds
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| serde::de::Error::custom("expires_in must be a positive number or string"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QrCredential {
    #[serde(rename = "appId")]
    pub app_id: String,
    #[serde(rename = "appSecret")]
    pub app_secret: String,
    #[serde(default, rename = "userOpenid")]
    pub user_openid: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HelperEvent {
    QrUrl { url: String },
    Expired,
    Credentials { credentials: Vec<QrCredential> },
    Error { message: String },
}

/// Run the official SDK helper and persist exactly one returned bot.
///
/// The helper communicates with this function using JSON Lines. Credential
/// lines are consumed but never printed. QR URLs are intentionally printed so
/// this CLI remains useful on a headless server where an image renderer is not
/// available.
pub async fn setup(secrets_path: &Path, secrets: &mut Secrets) -> Result<()> {
    setup_with_callback(secrets_path, secrets, None).await
}

pub async fn setup_with_callback(
    secrets_path: &Path,
    secrets: &mut Secrets,
    callback: Option<QqEventCallback>,
) -> Result<()> {
    let helper = std::env::var_os("QQ_SETUP_HELPER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            secrets_path
                .parent()
                .and_then(Path::parent)
                .map(|root| root.join("scripts/qq-setup.mjs"))
                .unwrap_or_else(|| PathBuf::from("scripts/qq-setup.mjs"))
        });
    if !helper.is_file() {
        bail!(
            "QQ setup helper not found at {}; run npm install --prefix scripts first",
            helper.display()
        );
    }

    let node_version = Command::new("node")
        .arg("--version")
        .output()
        .await
        .context("check Node.js >=18")?;
    if !node_version.status.success() {
        bail!("Node.js >=18 is required for QQ QR binding");
    }
    let version = String::from_utf8_lossy(&node_version.stdout);
    let major = version
        .trim()
        .strip_prefix('v')
        .and_then(|version| version.split('.').next())
        .and_then(|major| major.parse::<u64>().ok())
        .unwrap_or(0);
    if major < 18 {
        bail!(
            "Node.js >=18 is required for QQ QR binding, found {}",
            version.trim()
        );
    }

    let mut child = Command::new("node")
        .arg(&helper)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("start Node.js 18+ QQ QR helper")?;
    let stdout = child.stdout.take().context("capture QQ helper stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut credentials = None;

    while let Some(line) = lines.next_line().await? {
        let event: HelperEvent = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(_) => continue,
        };
        match event {
            HelperEvent::QrUrl { url } => {
                println!("QQ QR link: {url}");
                if let Some(callback) = &callback {
                    callback(QqEvent::QrUrl(url));
                }
            }
            HelperEvent::Expired => {
                println!("QQ QR expired; waiting for the refreshed QR link...");
                if let Some(callback) = &callback {
                    callback(QqEvent::QrExpired);
                }
            }
            HelperEvent::Credentials { credentials: found } => credentials = Some(found),
            HelperEvent::Error { message } => {
                let _ = child.wait().await;
                bail!("QQ QR binding failed: {message}");
            }
        }
    }
    let status = child.wait().await.context("wait for QQ QR helper")?;
    if !status.success() {
        bail!("QQ QR helper exited with {status}");
    }
    let found = credentials.context("QQ QR helper returned no credentials")?;
    if found.len() != 1 {
        bail!(
            "expected exactly one QQ bot credential, received {}",
            found.len()
        );
    }
    let credential = &found[0];
    if credential.app_id.trim().is_empty() || credential.app_secret.trim().is_empty() {
        bail!("QQ QR result is missing appId or appSecret");
    }

    secrets.qq_app_id = Some(credential.app_id.clone());
    secrets.qq_app_secret = Some(credential.app_secret.clone());
    secrets.qq_access_token = None;
    secrets.qq_access_token_expires_at = None;
    save_secrets(secrets_path, secrets)?;
    println!("QQ Bot bound; credentials were saved to the protected secrets file.");
    Ok(())
}

/// Get or refresh an access token. Expiry is treated conservatively with a
/// one-minute clock-skew margin and accepts the API's numeric or string form.
pub async fn access_token(
    client: &Client,
    secrets: &mut Secrets,
    secrets_path: &Path,
) -> Result<String> {
    if let (Some(token), Some(expires_at)) = (
        secrets.qq_access_token.clone(),
        secrets.qq_access_token_expires_at,
    ) {
        if Utc::now().timestamp() < expires_at.saturating_sub(60) {
            return Ok(token);
        }
    }
    let app_id = secrets.qq_app_id.as_deref().context("QQ app id missing")?;
    let app_secret = secrets
        .qq_app_secret
        .as_deref()
        .context("QQ app secret missing")?;
    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&json!({ "appId": app_id, "clientSecret": app_secret }))
        .send()
        .await
        .context("request QQ access token")?;
    let status = response.status();
    if !status.is_success() {
        bail!("QQ access token request failed with HTTP {status}");
    }
    let token: TokenResponse = response.json().await.context("decode QQ access token")?;
    let expires_at = Utc::now()
        .timestamp()
        .saturating_add(token.expires_in.max(60));
    secrets.qq_access_token = Some(token.access_token.clone());
    secrets.qq_access_token_expires_at = Some(expires_at);
    save_secrets(secrets_path, secrets)?;
    Ok(token.access_token)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QqEvent {
    QrUrl(String),
    QrExpired,
    TargetDiscovered(QqTarget),
    Incoming(QqIncomingMessage),
    GatewayConnected,
    GatewayDisconnected(String),
}

pub type QqEventCallback = Arc<dyn Fn(QqEvent) + Send + Sync>;
pub type ReadOnlyCommandHandler = Arc<dyn Fn(ReadOnlyCommand) -> Option<String> + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QqIncomingMessage {
    pub content: String,
    pub target: QqTarget,
    pub user_openid: Option<String>,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyCommand {
    Status,
    Holdings,
    Pnl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HoldingReport {
    pub symbol: String,
    pub name: String,
    pub shares: f64,
    pub average_cost: f64,
    pub last_price: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortfolioReport {
    pub cash: f64,
    pub total_assets: f64,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub holdings: Vec<HoldingReport>,
    pub ai_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TradeReport {
    pub action: String,
    pub symbol: String,
    pub name: String,
    pub shares: f64,
    pub price: f64,
    pub amount: f64,
    pub realized_pnl: Option<f64>,
    pub ai_reason: Option<String>,
    pub risk_note: Option<String>,
}

#[derive(Clone)]
pub struct QqService {
    client: Client,
    config: QqConfig,
    secrets: Arc<RwLock<Secrets>>,
    secrets_path: PathBuf,
    targets: Arc<RwLock<Vec<QqTarget>>>,
    pairing_token: Arc<RwLock<Option<String>>>,
    state: Arc<RwLock<QqBindingState>>,
    events: Option<QqEventCallback>,
    read_only_handler: Arc<RwLock<Option<ReadOnlyCommandHandler>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QqBindingState {
    pub qr_url: Option<String>,
    pub qr_expired: bool,
    pub last_discovered_target: Option<QqTarget>,
}

impl QqService {
    pub fn new(
        client: Client,
        config: QqConfig,
        secrets: Arc<RwLock<Secrets>>,
        secrets_path: PathBuf,
    ) -> Self {
        Self::with_events(client, config, secrets, secrets_path, None)
    }

    pub fn with_events(
        client: Client,
        config: QqConfig,
        secrets: Arc<RwLock<Secrets>>,
        secrets_path: PathBuf,
        events: Option<QqEventCallback>,
    ) -> Self {
        let targets = config.targets.clone();
        Self {
            client,
            config,
            secrets,
            secrets_path,
            targets: Arc::new(RwLock::new(targets)),
            pairing_token: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(QqBindingState::default())),
            events,
            read_only_handler: Arc::new(RwLock::new(None)),
        }
    }

    pub fn binding_state(&self) -> QqBindingState {
        self.state.read().clone()
    }

    pub fn set_read_only_handler(&self, handler: ReadOnlyCommandHandler) {
        *self.read_only_handler.write() = Some(handler);
    }

    pub fn targets(&self) -> Vec<QqTarget> {
        self.targets.read().clone()
    }

    fn is_authorized_target(&self, target: &QqTarget) -> bool {
        self.targets
            .read()
            .iter()
            .any(|existing| same_target(existing, target))
    }

    /// Issue a one-use token for the parent CLI/Web pairing flow.
    pub fn issue_pairing_token(&self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        *self.pairing_token.write() = Some(token.clone());
        token
    }

    /// Consume the pairing token and add the discovered QQ openid target.
    pub fn bind_target(&self, token: &str, target: QqTarget) -> bool {
        let accepted = self
            .pairing_token
            .write()
            .take()
            .is_some_and(|expected| expected == token);
        if !accepted {
            return false;
        }
        let mut targets = self.targets.write();
        if !targets
            .iter()
            .any(|existing| same_target(existing, &target))
        {
            targets.push(target);
        }
        true
    }

    pub async fn setup_qr(&self) -> Result<()> {
        let mut updated = self.secrets.read().clone();
        let service = self.clone();
        setup_with_callback(
            &self.secrets_path,
            &mut updated,
            Some(Arc::new(move |event| service.emit(event))),
        )
        .await?;
        *self.secrets.write() = updated;
        Ok(())
    }

    pub async fn access_token(&self) -> Result<String> {
        if let Some(token) = self.cached_token() {
            return Ok(token);
        }
        let snapshot = self.secrets.read().clone();
        let mut updated = snapshot;
        let token = access_token(&self.client, &mut updated, &self.secrets_path).await?;
        *self.secrets.write() = updated;
        Ok(token)
    }

    fn cached_token(&self) -> Option<String> {
        let secrets = self.secrets.read();
        match (&secrets.qq_access_token, secrets.qq_access_token_expires_at) {
            (Some(token), Some(expires_at))
                if Utc::now().timestamp() < expires_at.saturating_sub(60) =>
            {
                Some(token.clone())
            }
            _ => None,
        }
    }

    pub async fn gateway_url(&self) -> Result<String> {
        let token = self.access_token().await?;
        let base = api_base_url(&self.config);
        let response = self
            .client
            .get(format!("{base}{GATEWAY_URL_PATH}"))
            .headers(auth_headers(&token))
            .send()
            .await
            .context("request QQ Gateway URL")?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            bail!("QQ Gateway URL request was rate limited")
        }
        let response = response
            .error_for_status()
            .context("QQ Gateway URL HTTP error")?;
        let value: Value = response.json().await.context("decode QQ Gateway URL")?;
        value
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("QQ Gateway response missing url")
    }

    pub async fn send_text(&self, text: &str) -> Result<()> {
        let targets = self.targets();
        if targets.is_empty() {
            bail!("QQ message target is not configured");
        }
        for target in targets {
            self.send_target(&target, text).await?;
        }
        Ok(())
    }

    pub async fn send_portfolio(&self, report: &PortfolioReport) -> Result<()> {
        let text = render_portfolio(report, self.config.markdown);
        self.send_text(&text).await
    }

    pub async fn send_trade(&self, report: &TradeReport) -> Result<()> {
        self.send_text(&render_trade(report, self.config.markdown))
            .await
    }

    pub async fn send_target(&self, target: &QqTarget, text: &str) -> Result<()> {
        let token = self.access_token().await?;
        for chunk in split_message(text, MAX_MESSAGE_CHUNK_CHARS) {
            let request = message_request(&self.client, &token, &self.config, target, &chunk)?;
            let response = request.send().await.context("send QQ message")?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                let wait = retry_after(&response).unwrap_or(1);
                bail!("QQ message rate limited; retry after {wait}s (message was not retried)");
            }
            if response.status().is_server_error() {
                bail!(
                    "QQ message server error {} (message was not retried)",
                    response.status()
                );
            }
            response
                .error_for_status()
                .context("QQ message HTTP error")?;
        }
        Ok(())
    }

    /// Parse a Gateway `MESSAGE_CREATE` event. Direct messages use the official
    /// user openid; a QQ number or the legacy author id is never accepted.
    pub fn parse_incoming(payload: &Value) -> Option<QqIncomingMessage> {
        let content = payload.get("content")?.as_str()?.trim().to_owned();
        let message_id = payload.get("id").and_then(Value::as_str).map(str::to_owned);
        if let Some(group_openid) = payload.get("group_openid").and_then(Value::as_str) {
            return Some(QqIncomingMessage {
                content,
                target: QqTarget::Group {
                    openid: group_openid.to_owned(),
                },
                user_openid: payload
                    .pointer("/author/member/openid")
                    .or_else(|| payload.pointer("/author/user_openid"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                message_id,
            });
        }
        if let Some(channel_id) = payload.get("channel_id").and_then(Value::as_str) {
            return Some(QqIncomingMessage {
                content,
                target: QqTarget::Channel {
                    id: channel_id.to_owned(),
                },
                user_openid: payload
                    .pointer("/author/user_openid")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                message_id,
            });
        }
        let user_openid = payload
            .pointer("/author/user_openid")
            .or_else(|| payload.get("user_openid"))
            .and_then(Value::as_str)?;
        Some(QqIncomingMessage {
            content,
            target: QqTarget::User {
                openid: user_openid.to_owned(),
            },
            user_openid: Some(user_openid.to_owned()),
            message_id,
        })
    }

    pub async fn handle_incoming(&self, payload: &Value) -> Result<Option<QqIncomingMessage>> {
        let Some(message) = Self::parse_incoming(payload) else {
            return Ok(None);
        };
        self.emit(QqEvent::TargetDiscovered(message.target.clone()));
        self.emit(QqEvent::Incoming(message.clone()));
        let mut words = message.content.split_whitespace();
        let command = match words.next() {
            Some("/status") => Some(ReadOnlyCommand::Status),
            Some("/holdings") => Some(ReadOnlyCommand::Holdings),
            Some("/pnl") => Some(ReadOnlyCommand::Pnl),
            Some("/bind") => {
                if let Some(token) = words.next() {
                    if self.bind_target(token, message.target.clone()) {
                        self.send_target(&message.target, "QQ target bound for read-only reports.")
                            .await?;
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(command) = command {
            if self.is_authorized_target(&message.target) {
                let reply = self
                    .read_only_handler
                    .read()
                    .as_ref()
                    .and_then(|handler| handler(command));
                if let Some(reply) = reply {
                    self.send_target(&message.target, &reply).await?;
                }
            }
        }
        Ok(Some(message))
    }

    fn emit(&self, event: QqEvent) {
        match &event {
            QqEvent::QrUrl(url) => {
                let mut state = self.state.write();
                state.qr_url = Some(url.clone());
                state.qr_expired = false;
            }
            QqEvent::QrExpired => self.state.write().qr_expired = true,
            QqEvent::TargetDiscovered(target) => {
                self.state.write().last_discovered_target = Some(target.clone());
            }
            _ => {}
        }
        if let Some(callback) = &self.events {
            callback(event);
        }
    }

    /// Run the Gateway state machine with a caller-provided WebSocket adapter.
    /// The adapter is intentionally external because this repository currently
    /// does not declare `tokio-tungstenite` in Cargo.toml.
    pub async fn run_gateway<T, C, Fut>(&self, mut connect: C) -> Result<()>
    where
        T: GatewayTransport,
        C: FnMut(String) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut resume = GatewayResume::default();
        let mut backoff = Duration::from_secs(1);
        loop {
            let gateway_url = match resume.gateway_url.clone() {
                Some(url) => url,
                None => match self.gateway_url().await {
                    Ok(url) => url,
                    Err(error) => {
                        self.emit(QqEvent::GatewayDisconnected(error.to_string()));
                        sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        continue;
                    }
                },
            };
            let token = match self.access_token().await {
                Ok(token) => token,
                Err(error) => {
                    self.emit(QqEvent::GatewayDisconnected(error.to_string()));
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
            };
            let mut transport = match connect(gateway_url).await {
                Ok(transport) => transport,
                Err(error) => {
                    self.emit(QqEvent::GatewayDisconnected(error.to_string()));
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
            };
            match run_gateway_connection(self, &mut transport, &token, &mut resume).await {
                Ok(GatewayExit::Reconnect) => {}
                Ok(GatewayExit::InvalidSession) => resume = GatewayResume::default(),
                Err(error) => self.emit(QqEvent::GatewayDisconnected(error.to_string())),
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }
}

fn same_target(left: &QqTarget, right: &QqTarget) -> bool {
    match (left, right) {
        (QqTarget::Channel { id: left }, QqTarget::Channel { id: right }) => left == right,
        (QqTarget::Group { openid: left }, QqTarget::Group { openid: right }) => left == right,
        (QqTarget::User { openid: left }, QqTarget::User { openid: right }) => left == right,
        _ => false,
    }
}

fn api_base_url(config: &QqConfig) -> String {
    let base = if config.api_base_url.trim().is_empty() {
        DEFAULT_API_BASE_URL
    } else {
        config.api_base_url.trim()
    };
    base.trim_end_matches('/').to_owned()
}

fn auth_headers(token: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("QQBot {token}")
            .parse()
            .expect("valid QQ auth header"),
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    headers.insert(reqwest::header::ACCEPT, "application/json".parse().unwrap());
    headers
}

fn valid_target_id(id: &str) -> Result<&str> {
    if id.is_empty() || id.contains('/') || id.contains('?') || id.contains('#') {
        bail!("invalid QQ target id");
    }
    Ok(id)
}

fn message_request(
    client: &Client,
    token: &str,
    config: &QqConfig,
    target: &QqTarget,
    text: &str,
) -> Result<reqwest::RequestBuilder> {
    let base = api_base_url(config);
    let (path, body) = match target {
        QqTarget::Channel { id } => (
            format!("/channels/{}/messages", valid_target_id(id)?),
            if config.markdown {
                json!({ "msg_type": 2, "markdown": { "content": text } })
            } else {
                json!({ "content": text })
            },
        ),
        QqTarget::Group { openid } => (
            format!("/v2/groups/{}/messages", valid_target_id(openid)?),
            json!({ "content": text, "msg_type": 0 }),
        ),
        QqTarget::User { openid } => (
            format!("/v2/users/{}/messages", valid_target_id(openid)?),
            json!({ "content": text, "msg_type": 0 }),
        ),
    };
    Ok(client
        .post(format!("{base}{path}"))
        .headers(auth_headers(token))
        .json(&body))
}

fn retry_after(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

pub fn split_message(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() || max_chars == 0 {
        return vec![];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let candidate = if current.is_empty() {
            line.to_owned()
        } else {
            format!("{current}\n{line}")
        };
        if candidate.chars().count() <= max_chars {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        let mut part = String::new();
        for character in line.chars() {
            part.push(character);
            if part.chars().count() >= max_chars {
                chunks.push(std::mem::take(&mut part));
            }
        }
        current = part;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

pub fn render_portfolio(report: &PortfolioReport, markdown: bool) -> String {
    let title = if markdown {
        "## Portfolio"
    } else {
        "Portfolio"
    };
    let mut output = format!(
        "{title}\nCash: {:.2}\nTotal assets: {:.2}\nRealized P&L: {:+.2}\nUnrealized P&L: {:+.2}",
        report.cash, report.total_assets, report.realized_pnl, report.unrealized_pnl
    );
    if let Some(summary) = &report.ai_summary {
        output.push_str(&format!("\nAI summary: {summary}"));
    }
    if !report.holdings.is_empty() {
        output.push_str("\n\nHoldings:");
        for holding in &report.holdings {
            output.push_str(&format!(
                "\n- {} {}: {:.2} shares, cost {:.2}, last {:.2}, P&L {:+.2}",
                holding.symbol,
                holding.name,
                holding.shares,
                holding.average_cost,
                holding.last_price,
                holding.unrealized_pnl
            ));
        }
    }
    output
}

pub fn render_trade(report: &TradeReport, markdown: bool) -> String {
    let title = if markdown { "## Trade" } else { "Trade" };
    let mut output = format!(
        "{title}\n{} {} {} shares @ {:.2}\nAmount: {:.2}",
        report.action, report.symbol, report.shares, report.price, report.amount
    );
    if let Some(pnl) = report.realized_pnl {
        output.push_str(&format!("\nRealized P&L: {pnl:+.2}"));
    }
    if let Some(reason) = &report.ai_reason {
        output.push_str(&format!("\nAI reason: {reason}"));
    }
    if let Some(note) = &report.risk_note {
        output.push_str(&format!("\nRisk: {note}"));
    }
    output
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayResume {
    pub session_id: Option<String>,
    pub sequence: Option<i64>,
    pub gateway_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GatewayFrame {
    op: i64,
    #[serde(default)]
    d: Value,
    #[serde(default)]
    s: Option<i64>,
    #[serde(default)]
    t: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayExit {
    Reconnect,
    InvalidSession,
}

/// Minimal transport contract for a real WebSocket implementation.
/// `tokio-tungstenite` can implement it with text frames in a few lines.
pub struct TungsteniteTransport {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}
impl TungsteniteTransport {
    pub async fn connect(url: String) -> Result<Self> {
        let (socket, _) = connect_async(url)
            .await
            .context("connect QQ Gateway WebSocket")?;
        Ok(Self { socket })
    }
}
impl GatewayTransport for TungsteniteTransport {
    fn send_text<'a>(
        &'a mut self,
        text: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.socket
                .send(Message::Text(text.into()))
                .await
                .context("send QQ Gateway frame")?;
            Ok(())
        })
    }
    fn recv_text<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>> {
        Box::pin(async move {
            while let Some(message) = self.socket.next().await {
                match message.context("receive QQ Gateway frame")? {
                    Message::Text(text) => return Ok(Some(text.to_string())),
                    Message::Binary(bytes) => {
                        return Ok(Some(
                            String::from_utf8(bytes.to_vec())
                                .context("QQ Gateway binary frame is not UTF-8")?,
                        ));
                    }
                    Message::Ping(bytes) => {
                        self.socket.send(Message::Pong(bytes)).await?;
                    }
                    Message::Close(_) => return Ok(None),
                    _ => {}
                }
            }
            Ok(None)
        })
    }
}

pub async fn run_default_gateway(service: &QqService) -> Result<()> {
    let service = service.clone();
    service
        .run_gateway(|url| async move { TungsteniteTransport::connect(url).await })
        .await
}

pub trait GatewayTransport: Send {
    fn send_text<'a>(
        &'a mut self,
        text: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn recv_text<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>>;
}

async fn run_gateway_connection<T: GatewayTransport>(
    service: &QqService,
    transport: &mut T,
    token: &str,
    resume: &mut GatewayResume,
) -> Result<GatewayExit> {
    let hello = transport
        .recv_text()
        .await?
        .context("QQ Gateway closed before Hello")?;
    let hello: GatewayFrame = serde_json::from_str(&hello).context("decode QQ Gateway Hello")?;
    if hello.op != 10 {
        bail!(
            "QQ Gateway first frame was opcode {}, expected Hello",
            hello.op
        );
    }
    let heartbeat_ms = hello
        .d
        .get("heartbeat_interval")
        .and_then(Value::as_u64)
        .unwrap_or(41_250);
    if let (Some(session_id), Some(sequence)) = (&resume.session_id, resume.sequence) {
        transport
            .send_text(
                json!({
                    "op": 6,
                    "d": {
                        "token": token,
                        "session_id": session_id,
                        "seq": sequence
                    }
                })
                .to_string(),
            )
            .await?;
    } else {
        transport
            .send_text(
                json!({
                    "op": 2,
                    "d": {
                        "token": token,
                        "intents": gateway_intents(),
                        "shard": [0, 1],
                        "properties": {"os": "linux", "browser": "bilicraft-ai-stock", "device": "bilicraft-ai-stock"}
                    }
                })
                .to_string(),
            )
            .await?;
    }
    service.emit(QqEvent::GatewayConnected);
    let mut heartbeat = interval(Duration::from_millis(heartbeat_ms.max(1)));
    heartbeat.tick().await;
    let mut missed_heartbeats = 0u8;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if missed_heartbeats >= 2 {
                    bail!("QQ Gateway heartbeat acknowledgement timed out");
                }
                transport.send_text(json!({"op": 1, "d": resume.sequence}).to_string()).await?;
                missed_heartbeats = missed_heartbeats.saturating_add(1);
            }
            frame = transport.recv_text() => {
                let Some(frame) = frame? else { bail!("QQ Gateway connection closed") };
                let frame: GatewayFrame = serde_json::from_str(&frame).context("decode QQ Gateway frame")?;
                if let Some(sequence) = frame.s { resume.sequence = Some(sequence); }
                match frame.op {
                    0 => {
                        if frame.t.as_deref() == Some("READY") {
                            resume.session_id = frame.d.get("session_id").and_then(Value::as_str).map(str::to_owned);
                            resume.gateway_url = frame.d.get("resume_gateway_url").and_then(Value::as_str).map(str::to_owned);
                        }
                        if frame.t.as_deref() == Some("MESSAGE_CREATE") {
                            service.handle_incoming(&frame.d).await?;
                        }
                    }
                    1 => transport.send_text(json!({"op": 1, "d": resume.sequence}).to_string()).await?,
                    11 => missed_heartbeats = 0,
                    7 => return Ok(GatewayExit::Reconnect),
                    9 => return Ok(GatewayExit::InvalidSession),
                    _ => {}
                }
            }
        }
    }
}

pub fn gateway_intents() -> u64 {
    // GUILDS | GUILD_MESSAGES | DIRECT_MESSAGE | GROUP_AND_C2C_MESSAGE.
    (1 << 0) | (1 << 9) | (1 << 12) | (1 << 25)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_accepts_number_and_string() {
        let numeric: TokenResponse = serde_json::from_value(json!({
            "access_token": "x", "expires_in": 7200
        }))
        .unwrap();
        let string: TokenResponse = serde_json::from_value(json!({
            "access_token": "x", "expires_in": "7200"
        }))
        .unwrap();
        assert_eq!(numeric.expires_in, string.expires_in);
    }

    #[test]
    fn message_targets_use_official_routes_and_auth_prefix() {
        let client = Client::new();
        let config = QqConfig {
            enabled: true,
            api_base_url: "https://example.test/".into(),
            targets: vec![],
            markdown: false,
        };
        let request = message_request(
            &client,
            "token",
            &config,
            &QqTarget::Group {
                openid: "group-openid".into(),
            },
            "hello",
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://example.test/v2/groups/group-openid/messages"
        );
        assert_eq!(request.headers()["authorization"], "QQBot token");
    }

    #[test]
    fn parse_direct_message_requires_user_openid() {
        let message = QqService::parse_incoming(&json!({
            "content": "/status",
            "author": {"id": "qq-number-like-id"}
        }));
        assert!(message.is_none());
        let message = QqService::parse_incoming(&json!({
            "content": "/status",
            "author": {"user_openid": "openid"}
        }))
        .unwrap();
        assert!(matches!(message.target, QqTarget::User { openid } if openid == "openid"));
    }

    #[test]
    fn send_chunks_without_empty_lines() {
        assert_eq!(
            split_message("a\nb\nc", 3),
            vec!["a\nb".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn templates_include_detailed_portfolio_and_trade_fields() {
        let portfolio = render_portfolio(
            &PortfolioReport {
                cash: 10.0,
                total_assets: 20.0,
                realized_pnl: 1.0,
                unrealized_pnl: 2.0,
                holdings: vec![HoldingReport {
                    symbol: "ABC".into(),
                    name: "Alpha".into(),
                    shares: 3.0,
                    average_cost: 4.0,
                    last_price: 5.0,
                    unrealized_pnl: 3.0,
                }],
                ai_summary: Some("hold".into()),
            },
            true,
        );
        assert!(portfolio.contains("ABC Alpha"));
        assert!(portfolio.contains("AI summary"));
        let trade = render_trade(
            &TradeReport {
                action: "buy".into(),
                symbol: "ABC".into(),
                name: "Alpha".into(),
                shares: 1.0,
                price: 5.0,
                amount: 5.0,
                realized_pnl: None,
                ai_reason: Some("signal".into()),
                risk_note: Some("within budget".into()),
            },
            false,
        );
        assert!(trade.contains("AI reason: signal"));
        assert!(trade.contains("Risk: within budget"));
    }
}
