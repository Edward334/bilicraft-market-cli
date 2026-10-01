use crate::config::{Secrets, save_secrets};
use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEVICE_CODE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_ENTITLEMENTS: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const MC_PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";
const CLIENT_ID: &str = "c36a9fb6-4f2a-41ff-90bd-ae7cc92031eb";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub uuid: String,
    pub name: String,
    pub access_token: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeInfo {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

pub async fn request_device_code(client: &Client) -> Result<DeviceCodeInfo> {
    Ok(client
        .post(DEVICE_CODE)
        .form(&[
            ("client_id", CLIENT_ID),
            ("scope", "XboxLive.signin offline_access"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
async fn poll_token(client: &Client, info: &DeviceCodeInfo) -> Result<TokenResponse> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(info.expires_in);
    let mut interval = info.interval;
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if tokio::time::Instant::now() > deadline {
            return Err(anyhow!("Microsoft device code expired"));
        }
        let value: serde_json::Value = client
            .post(TOKEN)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", CLIENT_ID),
                ("device_code", &info.device_code),
            ])
            .send()
            .await?
            .json()
            .await?;
        if let Some(access_token) = value.get("access_token").and_then(|v| v.as_str()) {
            return Ok(TokenResponse {
                access_token: access_token.into(),
                refresh_token: value
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
            });
        }
        match value.get("error").and_then(|v| v.as_str()) {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some(e) => return Err(anyhow!("Microsoft login failed: {e}")),
            None => return Err(anyhow!("invalid Microsoft response")),
        }
    }
}
pub async fn complete_device_login(
    client: &Client,
    info: &DeviceCodeInfo,
    secrets_path: &std::path::Path,
    secrets: &mut Secrets,
) -> Result<Session> {
    let token = poll_token(client, info).await?;
    let ms = token.access_token;
    let xblv = client.post(XBL).json(&serde_json::json!({"Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={ms}")},"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"})).send().await?.error_for_status()?.json::<serde_json::Value>().await?;
    let xt = xblv["Token"].as_str().context("missing XBL token")?;
    let uhs = xblv["DisplayClaims"]["xui"][0]["uhs"]
        .as_str()
        .context("missing XBL uhs")?;
    let xj: serde_json::Value = client.post(XSTS).json(&serde_json::json!({"Properties":{"SandboxId":"RETAIL","UserTokens":[xt]},"RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"})).send().await?.json().await?;
    let xsts = xj["Token"].as_str().context("XSTS failed")?;
    let mc = client
        .post(MC_LOGIN)
        .json(&serde_json::json!({"identityToken":format!("XBL3.0 x={uhs};{xsts}")}))
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let access = mc["access_token"]
        .as_str()
        .context("missing Minecraft access token")?
        .to_string();
    let ent = client
        .get(MC_ENTITLEMENTS)
        .bearer_auth(&access)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    if ent["items"]
        .as_array()
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        return Err(anyhow!("Microsoft account does not own Minecraft"));
    }
    let profile = client
        .get(MC_PROFILE)
        .bearer_auth(&access)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let uuid = profile["id"]
        .as_str()
        .context("missing player uuid")?
        .to_owned();
    let name = profile["name"]
        .as_str()
        .context("missing player name")?
        .to_owned();
    secrets.microsoft_refresh_token = token.refresh_token;
    secrets.minecraft_access_token = Some(access.clone());
    secrets.minecraft_uuid = Some(uuid.clone());
    secrets.minecraft_player_name = Some(name.clone());
    save_secrets(secrets_path, secrets)?;
    Ok(Session {
        uuid,
        name,
        access_token: access,
    })
}
pub async fn device_login(
    client: &Client,
    secrets_path: &std::path::Path,
    secrets: &mut Secrets,
) -> Result<Session> {
    let info = request_device_code(client).await?;
    println!(
        "请在浏览器打开 {} 并输入验证码 {}",
        info.verification_uri, info.user_code
    );
    complete_device_login(client, &info, secrets_path, secrets).await
}
