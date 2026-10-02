//! Minecraft Java protocol client.
//!
//! The packet layout and phase behavior are based on the Kotlin reference under
//! `/tmp/bilicraft-handheld-plan/app/src/main/java/com/bilicraft/handheld/protocol/`
//! (MinecraftClient.kt, PacketPalette.kt, PipelineCodecs.kt, McCrypto.kt, Nbt.kt,
//! ChatComponent.kt, and McTypes.kt). PLAY ids are copied from the reference's
//! MCCTeam/Minecraft-Console-Client PacketPalette tables and are deliberately
//! registered only for exact protocol ranges 767..=776.

use aes::Aes128;
use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use anyhow::{Context, Result, anyhow, bail};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use rsa::{
    Pkcs1v15Encrypt, RsaPublicKey,
    pkcs8::DecodePublicKey,
    rand_core::{OsRng, RngCore},
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    io::{Cursor, Read, Write},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{broadcast, mpsc, watch},
};

const MAX_FRAME: usize = 2 * 1024 * 1024;
const MAX_DECOMPRESSED: usize = 8 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 262_144;
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_ITEMS: usize = 65_536;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEvent {
    pub text: String,
    pub raw: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    LoggingIn,
    Connected,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Login,
    Configuration,
    Play,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayIds {
    pub chat: i32,
    pub command: i32,
    pub keep_alive: i32,
    pub keep_alive_serverbound: i32,
    pub system_chat: i32,
    pub player_chat: i32,
    pub join_game: i32,
    pub start_configuration: i32,
    pub acknowledge_configuration: i32,
}

#[derive(Debug, Clone)]
pub struct Palette {
    pub protocol: i32,
    ids: PlayIds,
}

impl Palette {
    /// Return an exact known palette. Unknown protocol numbers are rejected;
    /// using a nearby packet table can silently corrupt the connection.
    pub fn for_protocol(protocol: i32) -> Result<Self> {
        let ids = match protocol {
            767 => PlayIds {
                chat: 0x06,
                command: 0x05,
                keep_alive: 0x26,
                keep_alive_serverbound: 0x18,
                system_chat: 0x6c,
                player_chat: 0x39,
                join_game: 0x2b,
                start_configuration: 0x69,
                acknowledge_configuration: 0x0c,
            },
            768 | 769 => PlayIds {
                chat: 0x07,
                command: 0x06,
                keep_alive: 0x27,
                keep_alive_serverbound: 0x1a,
                system_chat: 0x73,
                player_chat: 0x3b,
                join_game: 0x2c,
                start_configuration: 0x70,
                acknowledge_configuration: 0x0e,
            },
            770 => PlayIds {
                chat: 0x07,
                command: 0x06,
                keep_alive: 0x26,
                keep_alive_serverbound: 0x1a,
                system_chat: 0x72,
                player_chat: 0x3a,
                join_game: 0x2b,
                start_configuration: 0x6f,
                acknowledge_configuration: 0x0e,
            },
            771 | 772 => PlayIds {
                chat: 0x08,
                command: 0x07,
                keep_alive: 0x26,
                keep_alive_serverbound: 0x1b,
                system_chat: 0x72,
                player_chat: 0x3a,
                join_game: 0x2b,
                start_configuration: 0x6f,
                acknowledge_configuration: 0x0f,
            },
            773 | 774 => PlayIds {
                chat: 0x08,
                command: 0x07,
                keep_alive: 0x2b,
                keep_alive_serverbound: 0x1b,
                system_chat: 0x77,
                player_chat: 0x3f,
                join_game: 0x30,
                start_configuration: 0x74,
                acknowledge_configuration: 0x0f,
            },
            775 | 776 => PlayIds {
                chat: 0x09,
                command: 0x08,
                keep_alive: 0x2c,
                keep_alive_serverbound: 0x1c,
                system_chat: 0x79,
                player_chat: 0x41,
                join_game: 0x31,
                start_configuration: 0x76,
                acknowledge_configuration: 0x10,
            },
            _ => bail!(
                "unsupported Minecraft Java protocol {}; exact mappings are 767-776",
                protocol
            ),
        };
        Ok(Self { protocol, ids })
    }

    pub fn play_ids(&self) -> PlayIds {
        self.ids
    }
    fn has_known_packs(&self) -> bool {
        self.protocol >= 766
    }
    fn components_are_nbt(&self) -> bool {
        self.protocol >= 765
    }
    fn has_chat_checksum(&self) -> bool {
        self.protocol >= 770
    }
}

/// Commands accepted by the session actor. Commands are intentionally sent
/// through one channel so encrypted writes remain ordered with keep-alive ACKs.
#[derive(Debug, Clone)]
pub enum SessionCommand {
    SendCommand(String),
    Disconnect,
}

#[derive(Debug)]
pub struct SessionHandle {
    pub commands: mpsc::Sender<SessionCommand>,
    pub events: broadcast::Sender<ChatEvent>,
    pub state: watch::Receiver<ConnectionState>,
    pub error: watch::Receiver<Option<String>>,
}

impl Clone for SessionHandle {
    fn clone(&self) -> Self {
        Self {
            commands: self.commands.clone(),
            events: self.events.clone(),
            state: self.state.clone(),
            error: self.error.clone(),
        }
    }
}

impl SessionHandle {
    pub async fn send_command(&self, command: impl Into<String>) -> Result<()> {
        self.commands
            .send(SessionCommand::SendCommand(command.into()))
            .await
            .map_err(|_| anyhow!("Minecraft session actor stopped"))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ChatEvent> {
        self.events.subscribe()
    }

    pub fn is_connected(&self) -> bool {
        *self.state.borrow() == ConnectionState::Connected
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.commands
            .send(SessionCommand::Disconnect)
            .await
            .map_err(|_| anyhow!("Minecraft session actor stopped"))
    }
}

/// Connect to an exact protocol. Pass protocol 0 to status-ping first and use
/// the server's advertised protocol when it is one of the known 767-776 values.
pub async fn connect(
    host: impl Into<String>,
    port: u16,
    protocol: i32,
    username: impl Into<String>,
    uuid: impl Into<String>,
    access_token: impl Into<String>,
) -> Result<SessionHandle> {
    let host = host.into();
    let protocol = if protocol == 0 {
        server_ping(&host, port).await?.protocol
    } else {
        protocol
    };
    let palette = Palette::for_protocol(protocol)?;
    let stream = TcpStream::connect((host.as_str(), port))
        .await
        .context("connect Minecraft server")?;
    let (commands, receiver) = mpsc::channel(64);
    let (events, _) = broadcast::channel(256);
    let (state_tx, state_rx) = watch::channel(ConnectionState::Connecting);
    let (error_tx, error_rx) = watch::channel(None);
    let session = SessionHandle {
        commands,
        events: events.clone(),
        state: state_rx,
        error: error_rx,
    };
    tokio::spawn(run_session(
        stream,
        host,
        port,
        palette,
        username.into(),
        uuid.into(),
        access_token.into(),
        receiver,
        events,
        state_tx,
        error_tx,
    ));
    Ok(session)
}

/// Query status and latency using the protocol-independent status handshake.
pub async fn server_ping(host: &str, port: u16) -> Result<ServerPing> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = TcpStream::connect((host, port))
            .await
            .context("connect for Minecraft status ping")?;
        let mut handshake = Vec::new();
        write_varint(0, &mut handshake);
        write_varint(-1, &mut handshake);
        write_string(host, &mut handshake)?;
        handshake.extend_from_slice(&port.to_be_bytes());
        write_varint(1, &mut handshake);
        write_raw_frame(&mut stream, &handshake).await?;
        write_raw_frame(&mut stream, &[0]).await?;
        let response = read_raw_frame(&mut stream).await?;
        let mut cursor = response.as_slice();
        ensure_id(&mut cursor, 0)?;
        let json = read_string(&mut cursor)?;
        let status: serde_json::Value =
            serde_json::from_str(&json).context("decode Minecraft status JSON")?;
        let version = status.get("version").context("status has no version")?;
        let protocol = version
            .get("protocol")
            .and_then(serde_json::Value::as_i64)
            .context("status has no numeric protocol")? as i32;
        let name = version
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let description = status
            .get("description")
            .map(|value| normalize_value(value.clone()))
            .unwrap_or_default();
        write_raw_frame(&mut stream, &[1, 0, 0, 0, 0, 0, 0, 0, 0]).await?;
        let pong = read_raw_frame(&mut stream).await?;
        let mut pong_cursor = pong.as_slice();
        ensure_id(&mut pong_cursor, 1)?;
        Ok(ServerPing {
            protocol,
            name,
            description,
        })
    })
    .await
    .context("Minecraft status ping timed out")?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPing {
    pub protocol: i32,
    pub name: String,
    pub description: String,
}

async fn run_session(
    mut stream: TcpStream,
    host: String,
    port: u16,
    palette: Palette,
    username: String,
    uuid: String,
    access_token: String,
    mut commands: mpsc::Receiver<SessionCommand>,
    events: broadcast::Sender<ChatEvent>,
    state: watch::Sender<ConnectionState>,
    error_state: watch::Sender<Option<String>>,
) {
    let result = session_inner(
        &mut stream,
        &host,
        port,
        &palette,
        &username,
        &uuid,
        &access_token,
        &mut commands,
        &events,
        &state,
    )
    .await;
    if let Err(error) = result {
        tracing::error!(error=?error, "Minecraft session actor stopped");
        let _ = error_state.send(Some(error.to_string()));
        let current = *state.borrow();
        let terminal = if current == ConnectionState::Connected {
            ConnectionState::Disconnected
        } else {
            ConnectionState::Failed
        };
        let _ = state.send(terminal);
    }
}

async fn session_inner(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    palette: &Palette,
    username: &str,
    uuid: &str,
    access_token: &str,
    commands: &mut mpsc::Receiver<SessionCommand>,
    events: &broadcast::Sender<ChatEvent>,
    state: &watch::Sender<ConnectionState>,
) -> Result<()> {
    let mut transport = Transport::default();
    let mut phase = Phase::Login;
    send_handshake_and_login(stream, palette.protocol, host, port, username, uuid).await?;
    let _ = state.send(ConnectionState::LoggingIn);

    loop {
        tokio::select! {
            packet = transport.read_packet(stream) => {
                let packet = packet?;
                handle_packet(stream, &mut transport, &mut phase, palette, username, access_token, uuid, packet, events, state).await?;
            }
            command = commands.recv() => match command {
                Some(SessionCommand::SendCommand(command)) => {
                    if phase == Phase::Play { transport.write_packet(stream, &encode_command(palette, &command)?).await?; }
                }
                Some(SessionCommand::Disconnect) | None => {
                    let _ = state.send(ConnectionState::Disconnected);
                    return Ok(());
                }
            }
        }
    }
}

async fn send_handshake_and_login(
    stream: &mut TcpStream,
    protocol: i32,
    host: &str,
    port: u16,
    username: &str,
    uuid: &str,
) -> Result<()> {
    let mut handshake = Vec::new();
    write_varint(0, &mut handshake);
    write_varint(protocol, &mut handshake);
    write_string(host, &mut handshake)?;
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(2, &mut handshake);
    write_raw_frame(stream, &handshake).await?;

    let mut login = Vec::new();
    write_varint(0, &mut login);
    write_string(username, &mut login)?;
    write_uuid(uuid, &mut login)?;
    write_raw_frame(stream, &login).await
}

async fn handle_packet(
    stream: &mut TcpStream,
    transport: &mut Transport,
    phase: &mut Phase,
    palette: &Palette,
    username: &str,
    access_token: &str,
    uuid: &str,
    packet: Vec<u8>,
    events: &broadcast::Sender<ChatEvent>,
    state: &watch::Sender<ConnectionState>,
) -> Result<()> {
    let mut cursor = packet.as_slice();
    let id = read_varint(&mut cursor)?;
    match *phase {
        Phase::Login => match id {
            0x00 => bail!(
                "Minecraft login disconnected: {}",
                read_login_component(&mut cursor)?
            ),
            0x01 => {
                let server_id = read_string(&mut cursor)?;
                let public_key = read_byte_array(&mut cursor)?;
                let verify_token = read_byte_array(&mut cursor)?;
                let secret = random_secret();
                let key = RsaPublicKey::from_public_key_der(&public_key)
                    .context("decode Minecraft RSA public key")?;
                let hash = server_hash(&server_id, &secret, &public_key);
                join_server(access_token, uuid, &hash).await?;
                let encrypted_secret = key
                    .encrypt(&mut OsRng, Pkcs1v15Encrypt, &secret)
                    .context("RSA encrypt shared secret")?;
                let encrypted_token = key
                    .encrypt(&mut OsRng, Pkcs1v15Encrypt, &verify_token)
                    .context("RSA encrypt verify token")?;
                let mut response = Vec::new();
                write_varint(0x01, &mut response);
                write_byte_array(&encrypted_secret, &mut response)?;
                write_byte_array(&encrypted_token, &mut response)?;
                transport.write_raw_packet(stream, &response).await?;
                transport.cipher = Some(Cfb8::new(&secret));
            }
            0x02 => {
                let _ = read_uuid(&mut cursor)?;
                let _ = read_string(&mut cursor)?;
                *phase = Phase::Configuration;
                transport
                    .write_packet(stream, &packet_with_id(0x03))
                    .await?;
                send_client_information(stream, transport, palette).await?;
            }
            0x03 => transport.compression = Some(read_varint(&mut cursor)?),
            0x04 => {
                let message_id = read_varint(&mut cursor)?;
                let _channel = read_string(&mut cursor)?;
                let mut response = Vec::new();
                write_varint(0x02, &mut response);
                write_varint(message_id, &mut response);
                response.push(0);
                transport.write_packet(stream, &response).await?;
            }
            _ => {}
        },
        Phase::Configuration => match id {
            0x02 => bail!(
                "Minecraft configuration disconnected: {}",
                read_component_text(&mut cursor, palette.components_are_nbt())?
            ),
            0x03 => {
                transport
                    .write_packet(stream, &packet_with_id(0x03))
                    .await?;
                *phase = Phase::Play;
                let _ = state.send(ConnectionState::Connected);
            }
            0x04 => {
                let keep_alive = read_i64(&mut cursor)?;
                let mut response = Vec::new();
                write_varint(0x04, &mut response);
                response.extend_from_slice(&keep_alive.to_be_bytes());
                transport.write_packet(stream, &response).await?;
            }
            0x09 if palette.protocol >= 765 => {
                let request_id = read_uuid(&mut cursor)?;
                let _url = read_string(&mut cursor)?;
                let _hash = read_string(&mut cursor)?;
                let _required = read_bool(&mut cursor)?;
                if read_bool(&mut cursor)? {
                    let _ = read_component_text(&mut cursor, palette.components_are_nbt())?;
                }
                let mut response = Vec::new();
                write_varint(0x06, &mut response);
                write_uuid_bytes(request_id, &mut response);
                write_varint(1, &mut response); // DECLINED: this handle has no resource-pack fetch API.
                transport.write_packet(stream, &response).await?;
            }
            0x0e if palette.has_known_packs() => {
                let mut response = Vec::new();
                write_varint(0x07, &mut response);
                write_varint(0, &mut response);
                transport.write_packet(stream, &response).await?;
            }
            _ => {}
        },
        Phase::Play => {
            let ids = palette.play_ids();
            match id {
                x if x == ids.keep_alive => {
                    let keep_alive = read_i64(&mut cursor)?;
                    let mut response = Vec::new();
                    write_varint(ids.keep_alive_serverbound, &mut response);
                    response.extend_from_slice(&keep_alive.to_be_bytes());
                    transport.write_packet(stream, &response).await?;
                }
                x if x == ids.system_chat => {
                    let raw = read_component_text(&mut cursor, palette.components_are_nbt())?;
                    let _overlay = read_bool(&mut cursor)?;
                    emit_event(events, raw, None);
                }
                x if x == ids.player_chat => {
                    if let Some((sender, raw)) =
                        parse_player_chat(&mut cursor, palette.components_are_nbt())?
                    {
                        emit_event(events, raw, Some(sender));
                    }
                }
                x if x == ids.start_configuration => {
                    transport
                        .write_packet(stream, &packet_with_id(ids.acknowledge_configuration))
                        .await?;
                    *phase = Phase::Configuration;
                }
                _ => {}
            }
        }
    }
    let _ = username;
    Ok(())
}

async fn send_client_information(
    stream: &mut TcpStream,
    transport: &mut Transport,
    palette: &Palette,
) -> Result<()> {
    let mut packet = Vec::new();
    write_varint(0x00, &mut packet);
    write_string("zh_cn", &mut packet)?;
    packet.push(8);
    write_varint(0, &mut packet);
    packet.push(1);
    packet.push(0x7f);
    write_varint(1, &mut packet);
    packet.push(0);
    packet.push(1);
    if palette.protocol >= 768 {
        write_varint(0, &mut packet);
    }
    transport.write_packet(stream, &packet).await
}

fn encode_command(palette: &Palette, command: &str) -> Result<Vec<u8>> {
    let command = command.trim().trim_start_matches('/');
    if command.is_empty() {
        bail!("empty Minecraft command")
    }
    let mut packet = Vec::new();
    write_varint(palette.play_ids().command, &mut packet);
    write_string(command, &mut packet)?;
    packet.extend_from_slice(&chrono::Utc::now().timestamp_millis().to_be_bytes());
    packet.extend_from_slice(&0i64.to_be_bytes());
    write_varint(0, &mut packet); // parameter signatures
    write_varint(0, &mut packet); // last-seen message count
    packet.extend_from_slice(&[0; 3]); // fixed 20-bit acknowledged bit set
    if palette.has_chat_checksum() {
        packet.push(0);
    }
    Ok(packet)
}

async fn join_server(access_token: &str, uuid: &str, server_hash: &str) -> Result<()> {
    if access_token.is_empty() {
        bail!("online-mode server requires a Microsoft access token")
    }
    let profile = uuid.replace('-', "");
    let response = reqwest::Client::new()
        .post("https://sessionserver.mojang.com/session/minecraft/join")
        .json(&serde_json::json!({ "accessToken": access_token, "selectedProfile": profile, "serverId": server_hash }))
        .send().await.context("Minecraft session join request")?;
    if !response.status().is_success() {
        bail!(
            "Minecraft session join rejected with HTTP {}",
            response.status()
        )
    }
    Ok(())
}

fn random_secret() -> [u8; 16] {
    let mut secret = [0u8; 16];
    OsRng.fill_bytes(&mut secret);
    secret
}

/// Minecraft's server hash is SHA-1 interpreted as a signed big-endian integer.
fn server_hash(server_id: &str, secret: &[u8; 16], public_key_der: &[u8]) -> String {
    let mut digest = Sha1::new();
    digest.update(server_id.as_bytes());
    digest.update(secret);
    digest.update(public_key_der);
    signed_hex(&digest.finalize())
}

fn signed_hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0".into();
    }
    if bytes[0] & 0x80 == 0 {
        let first = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
        return if first == bytes.len() {
            "0".into()
        } else {
            hex_lower(&bytes[first..])
        };
    }
    let mut magnitude = bytes.to_vec();
    for byte in &mut magnitude {
        *byte = !*byte;
    }
    for byte in magnitude.iter_mut().rev() {
        let (next, carry) = byte.overflowing_add(1);
        *byte = next;
        if !carry {
            break;
        }
    }
    let first = magnitude
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(magnitude.len());
    format!(
        "-{}",
        if first == magnitude.len() {
            "0".into()
        } else {
            hex_lower(&magnitude[first..])
        }
    )
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[derive(Debug, Default)]
struct Transport {
    compression: Option<i32>,
    cipher: Option<Cfb8>,
}

impl Transport {
    async fn read_packet(&mut self, stream: &mut TcpStream) -> Result<Vec<u8>> {
        let length = self.read_varint(stream).await?;
        if length < 0 || length as usize > MAX_FRAME {
            bail!("invalid Minecraft frame length {}", length)
        }
        let mut body = vec![0u8; length as usize];
        self.read_exact(stream, &mut body).await?;
        if let Some(_threshold) = self.compression {
            let mut cursor = body.as_slice();
            let uncompressed = read_varint(&mut cursor)?;
            if uncompressed < 0 {
                bail!("negative compressed data length")
            }
            if uncompressed == 0 {
                return Ok(cursor.to_vec());
            }
            let expected = uncompressed as usize;
            if expected > MAX_DECOMPRESSED {
                bail!("Minecraft decompressed packet exceeds bound")
            }
            let decoder = ZlibDecoder::new(Cursor::new(cursor));
            let mut output = Vec::with_capacity(expected);
            decoder
                .take((expected + 1) as u64)
                .read_to_end(&mut output)?;
            if output.len() != expected {
                bail!("compressed packet length mismatch")
            }
            Ok(output)
        } else {
            Ok(body)
        }
    }

    async fn write_packet(&mut self, stream: &mut TcpStream, packet: &[u8]) -> Result<()> {
        let body = if let Some(threshold) = self.compression {
            if threshold < 0 {
                bail!("negative compression threshold")
            }
            let mut body = Vec::new();
            if packet.len() < threshold as usize {
                write_varint(0, &mut body);
                body.extend_from_slice(packet);
            } else {
                write_varint(packet.len() as i32, &mut body);
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(packet)?;
                body.extend_from_slice(&encoder.finish()?);
            }
            body
        } else {
            packet.to_vec()
        };
        self.write_raw_packet(stream, &body).await
    }

    async fn write_raw_packet(&mut self, stream: &mut TcpStream, body: &[u8]) -> Result<()> {
        if body.len() > MAX_FRAME {
            bail!("Minecraft packet exceeds bound")
        }
        let mut framed = Vec::with_capacity(body.len() + 5);
        write_varint(body.len() as i32, &mut framed);
        framed.extend_from_slice(body);
        self.write_all(stream, &mut framed).await
    }

    async fn read_varint(&mut self, stream: &mut TcpStream) -> Result<i32> {
        let mut bytes = [0u8; 5];
        for i in 0..5 {
            self.read_exact(stream, &mut bytes[i..i + 1]).await?;
            if bytes[i] & 0x80 == 0 {
                let mut slice = &bytes[..i + 1];
                return read_varint(&mut slice);
            }
        }
        bail!("Minecraft frame VarInt too long")
    }

    async fn read_exact(&mut self, stream: &mut TcpStream, data: &mut [u8]) -> Result<()> {
        stream.read_exact(data).await?;
        if let Some(cipher) = &mut self.cipher {
            cipher.decrypt(data);
        }
        Ok(())
    }

    async fn write_all(&mut self, stream: &mut TcpStream, data: &mut [u8]) -> Result<()> {
        if let Some(cipher) = &mut self.cipher {
            cipher.encrypt(data);
        }
        stream.write_all(data).await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Cfb8 {
    cipher: Aes128,
    register: [u8; 16],
}

impl Cfb8 {
    fn new(key: &[u8; 16]) -> Self {
        Self {
            cipher: Aes128::new(GenericArray::from_slice(key)),
            register: *key,
        }
    }

    fn process(&mut self, bytes: &mut [u8], decrypt: bool) {
        for byte in bytes {
            let mut block = GenericArray::clone_from_slice(&self.register);
            self.cipher.encrypt_block(&mut block);
            let input = *byte;
            *byte ^= block[0];
            self.register.copy_within(1.., 0);
            self.register[15] = if decrypt { input } else { *byte };
        }
    }

    fn encrypt(&mut self, bytes: &mut [u8]) {
        self.process(bytes, false);
    }
    fn decrypt(&mut self, bytes: &mut [u8]) {
        self.process(bytes, true);
    }
}

fn packet_with_id(id: i32) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(id, &mut out);
    out
}

pub fn read_varint(buf: &mut &[u8]) -> Result<i32> {
    let mut value = 0u32;
    for index in 0..5 {
        let byte = *buf.first().context("unexpected end of VarInt")?;
        *buf = &buf[1..];
        if index == 4 && byte & 0x70 != 0 {
            bail!("VarInt overflows 32 bits")
        }
        value |= ((byte & 0x7f) as u32) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    bail!("VarInt too long")
}

pub fn write_varint(value: i32, out: &mut Vec<u8>) {
    let mut unsigned = value as u32;
    loop {
        let mut byte = (unsigned & 0x7f) as u8;
        unsigned >>= 7;
        if unsigned != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if unsigned == 0 {
            break;
        }
    }
}

fn write_string(value: &str, out: &mut Vec<u8>) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() > MAX_STRING_BYTES {
        bail!("Minecraft string exceeds bound")
    }
    write_varint(bytes.len() as i32, out);
    out.extend_from_slice(bytes);
    Ok(())
}

fn read_string(input: &mut &[u8]) -> Result<String> {
    let length = read_varint(input)?;
    if length < 0 || length as usize > MAX_STRING_BYTES || input.len() < length as usize {
        bail!("invalid Minecraft string length")
    }
    let (bytes, rest) = input.split_at(length as usize);
    *input = rest;
    Ok(String::from_utf8(bytes.to_vec()).context("invalid UTF-8 Minecraft string")?)
}

fn write_byte_array(value: &[u8], out: &mut Vec<u8>) -> Result<()> {
    write_varint(value.len() as i32, out);
    out.extend_from_slice(value);
    Ok(())
}
fn read_byte_array(input: &mut &[u8]) -> Result<Vec<u8>> {
    let length = read_varint(input)?;
    if length < 0 || length as usize > MAX_DECOMPRESSED || input.len() < length as usize {
        bail!("invalid Minecraft byte-array length")
    }
    let (bytes, rest) = input.split_at(length as usize);
    *input = rest;
    Ok(bytes.to_vec())
}
fn read_bool(input: &mut &[u8]) -> Result<bool> {
    Ok(*input.first().context("missing boolean")? != 0).inspect(|_| *input = &input[1..])
}
fn read_i64(input: &mut &[u8]) -> Result<i64> {
    if input.len() < 8 {
        bail!("missing i64")
    };
    let (x, rest) = input.split_at(8);
    *input = rest;
    Ok(i64::from_be_bytes(x.try_into().unwrap()))
}
fn ensure_id(input: &mut &[u8], expected: i32) -> Result<()> {
    let id = read_varint(input)?;
    if id != expected {
        bail!("unexpected packet id {}; expected {}", id, expected)
    };
    Ok(())
}
fn read_uuid(input: &mut &[u8]) -> Result<[u8; 16]> {
    if input.len() < 16 {
        bail!("missing UUID")
    };
    let (x, rest) = input.split_at(16);
    *input = rest;
    Ok(x.try_into().unwrap())
}
fn write_uuid(value: &str, out: &mut Vec<u8>) -> Result<()> {
    let uuid = uuid::Uuid::parse_str(value).context("invalid player UUID")?;
    write_uuid_bytes(*uuid.as_bytes(), out);
    Ok(())
}
fn write_uuid_bytes(value: [u8; 16], out: &mut Vec<u8>) {
    out.extend_from_slice(&value);
}

async fn write_raw_frame(stream: &mut TcpStream, body: &[u8]) -> Result<()> {
    if body.len() > MAX_FRAME {
        bail!("frame exceeds bound")
    }
    let mut frame = Vec::new();
    write_varint(body.len() as i32, &mut frame);
    frame.extend_from_slice(body);
    stream.write_all(&frame).await?;
    Ok(())
}
async fn read_raw_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut first = [0u8; 1];
    let mut encoded = Vec::new();
    for _ in 0..5 {
        stream.read_exact(&mut first).await?;
        encoded.push(first[0]);
        if first[0] & 0x80 == 0 {
            break;
        }
    }
    let mut cursor = encoded.as_slice();
    let length = read_varint(&mut cursor)?;
    if length < 0 || length as usize > MAX_FRAME {
        bail!("invalid raw frame length")
    }
    let mut body = vec![0; length as usize];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

fn read_login_component(input: &mut &[u8]) -> Result<String> {
    Ok(normalize_chat(&read_string(input)?))
}
fn read_component_text(input: &mut &[u8], nbt: bool) -> Result<String> {
    if nbt {
        Ok(parse_network_nbt(input)?.to_plain())
    } else {
        Ok(normalize_chat(&read_string(input)?))
    }
}
fn emit_event(events: &broadcast::Sender<ChatEvent>, raw: String, sender: Option<String>) {
    let text = match sender {
        Some(sender) if !sender.is_empty() => format!("<{sender}> {}", normalize_chat(&raw)),
        _ => normalize_chat(&raw),
    };
    if text.trim().is_empty() {
        return;
    }
    let _ = events.send(ChatEvent {
        text,
        raw,
        timestamp: chrono::Utc::now().timestamp_millis(),
    });
}

fn parse_player_chat(input: &mut &[u8], nbt: bool) -> Result<Option<(String, String)>> {
    let _sender_uuid = read_uuid(input)?;
    let _index = read_varint(input)?;
    if read_bool(input)? {
        if input.len() < 256 {
            bail!("truncated player chat signature")
        };
        *input = &input[256..];
    }
    let body = read_string(input)?;
    let _timestamp = read_i64(input)?;
    let _salt = read_i64(input)?;
    let previous = read_varint(input)?;
    if previous < 0 || previous as usize > 256 {
        bail!("invalid previous-message count")
    }
    for _ in 0..previous {
        if read_varint(input)? == 0 {
            if input.len() < 256 {
                bail!("truncated previous signature")
            };
            *input = &input[256..];
        }
    }
    let unsigned = if read_bool(input)? {
        Some(read_component_text(input, nbt)?)
    } else {
        None
    };
    let filter = read_varint(input)?;
    if filter == 2 {
        let words = read_varint(input)?;
        if words < 0 || words as usize > 256 || input.len() < words as usize * 8 {
            bail!("invalid chat filter mask")
        };
        *input = &input[words as usize * 8..];
    }
    let _chat_type = read_varint(input)?;
    let sender = read_component_text(input, nbt)?;
    if read_bool(input)? {
        let _ = read_component_text(input, nbt)?;
    }
    Ok(Some((sender, unsigned.unwrap_or(body))))
}

#[derive(Debug, Clone, PartialEq)]
pub enum NbtValue {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    ByteArray(Vec<i8>),
    List(Vec<NbtValue>),
    Compound(Vec<(String, NbtValue)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl NbtValue {
    fn to_plain(&self) -> String {
        match self {
            NbtValue::String(value) => value.clone(),
            NbtValue::List(values) => values.iter().map(NbtValue::to_plain).collect(),
            NbtValue::Compound(entries) => {
                let get = |key: &str| {
                    entries
                        .iter()
                        .find(|(name, _)| name == key)
                        .map(|(_, value)| value)
                };
                let mut text = get("text").map(NbtValue::to_plain).unwrap_or_default();
                if let Some(translate) = get("translate").map(NbtValue::to_plain) {
                    let args = get("with").map(NbtValue::to_plain).unwrap_or_default();
                    text.push_str(&render_translation(&translate, &args));
                }
                if let Some(extra) = get("extra") {
                    text.push_str(&extra.to_plain());
                }
                text
            }
            _ => String::new(),
        }
    }
}

pub fn parse_network_nbt(input: &mut &[u8]) -> Result<NbtValue> {
    parse_nbt_tag(input, 0, false)
}

fn parse_nbt_tag(input: &mut &[u8], depth: usize, named: bool) -> Result<NbtValue> {
    if depth >= MAX_NBT_DEPTH {
        bail!("NBT nesting exceeds bound")
    }
    let kind = *input.first().context("missing NBT tag type")?;
    *input = &input[1..];
    if kind == 0 {
        return Ok(NbtValue::End);
    }
    if named {
        let _ = read_nbt_string(input)?;
    }
    parse_nbt_payload(input, kind, depth + 1)
}

fn parse_nbt_payload(input: &mut &[u8], kind: u8, depth: usize) -> Result<NbtValue> {
    match kind {
        1 => Ok(NbtValue::Byte(read_i8(input)?)),
        2 => Ok(NbtValue::Short(read_i16(input)?)),
        3 => Ok(NbtValue::Int(read_i32(input)?)),
        4 => Ok(NbtValue::Long(read_i64(input)?)),
        5 => Ok(NbtValue::Float(f32::from_bits(read_u32(input)?))),
        6 => Ok(NbtValue::Double(f64::from_bits(read_u64(input)?))),
        7 => {
            let len = bounded_array_len(input, 1)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(read_i8(input)?);
            }
            Ok(NbtValue::ByteArray(values))
        }
        8 => Ok(NbtValue::String(read_nbt_string(input)?)),
        9 => {
            let element = *input.first().context("missing NBT list type")?;
            *input = &input[1..];
            let len = read_i32(input)?;
            if len < 0 || len as usize > MAX_NBT_ITEMS {
                bail!("invalid NBT list length")
            }
            let mut values = Vec::with_capacity(len as usize);
            for _ in 0..len {
                values.push(parse_nbt_payload(input, element, depth)?);
            }
            Ok(NbtValue::List(values))
        }
        10 => {
            let mut entries = Vec::new();
            loop {
                let kind = *input.first().context("unterminated NBT compound")?;
                if kind == 0 {
                    *input = &input[1..];
                    break;
                }
                if entries.len() >= MAX_NBT_ITEMS {
                    bail!("NBT compound too large")
                };
                *input = &input[1..];
                let name = read_nbt_string(input)?;
                entries.push((name, parse_nbt_payload(input, kind, depth)?));
            }
            Ok(NbtValue::Compound(entries))
        }
        11 => {
            let len = bounded_array_len(input, 4)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(read_i32(input)?);
            }
            Ok(NbtValue::IntArray(values))
        }
        12 => {
            let len = bounded_array_len(input, 8)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(read_i64(input)?);
            }
            Ok(NbtValue::LongArray(values))
        }
        _ => bail!("unknown NBT tag type {}", kind),
    }
}

fn bounded_array_len(input: &mut &[u8], width: usize) -> Result<usize> {
    let len = read_i32(input)?;
    if len < 0 || len as usize > MAX_NBT_ITEMS || input.len() < len as usize * width {
        bail!("invalid NBT array length")
    };
    Ok(len as usize)
}
fn read_nbt_string(input: &mut &[u8]) -> Result<String> {
    let len = read_u16(input)? as usize;
    if input.len() < len {
        bail!("truncated NBT string")
    };
    let (bytes, rest) = input.split_at(len);
    *input = rest;
    decode_modified_utf8(bytes)
}
fn decode_modified_utf8(bytes: &[u8]) -> Result<String> {
    let mut units = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte < 0x80 {
            if byte == 0 {
                bail!("invalid modified UTF-8 NUL")
            };
            units.push(byte as u16);
            index += 1;
        } else if byte & 0xe0 == 0xc0 {
            if index + 1 >= bytes.len() {
                bail!("truncated modified UTF-8")
            };
            let value = (((byte & 0x1f) as u16) << 6) | (bytes[index + 1] & 0x3f) as u16;
            units.push(value);
            index += 2;
        } else if byte & 0xf0 == 0xe0 {
            if index + 2 >= bytes.len() {
                bail!("truncated modified UTF-8")
            };
            let value = (((byte & 0x0f) as u16) << 12)
                | (((bytes[index + 1] & 0x3f) as u16) << 6)
                | (bytes[index + 2] & 0x3f) as u16;
            units.push(value);
            index += 3;
        } else if byte & 0xf8 == 0xf0 {
            if index + 3 >= bytes.len() {
                bail!("truncated UTF-8")
            };
            let value = (((byte & 7) as u32) << 18)
                | (((bytes[index + 1] & 0x3f) as u32) << 12)
                | (((bytes[index + 2] & 0x3f) as u32) << 6)
                | (bytes[index + 3] & 0x3f) as u32;
            let ch = char::from_u32(value).context("invalid UTF-8 code point")?;
            let mut pair = [0u16; 2];
            ch.encode_utf16(&mut pair);
            units.extend_from_slice(&pair[..ch.len_utf16()]);
            index += 4;
        } else {
            bail!("invalid modified UTF-8")
        }
    }
    String::from_utf16(&units).context("invalid UTF-16 in modified UTF-8")
}

fn read_i8(input: &mut &[u8]) -> Result<i8> {
    Ok(*input.first().context("missing i8")? as i8).inspect(|_| *input = &input[1..])
}
fn read_i16(input: &mut &[u8]) -> Result<i16> {
    Ok(read_u16(input)? as i16)
}
fn read_i32(input: &mut &[u8]) -> Result<i32> {
    Ok(read_u32(input)? as i32)
}
fn read_u16(input: &mut &[u8]) -> Result<u16> {
    if input.len() < 2 {
        bail!("missing u16")
    };
    let (x, rest) = input.split_at(2);
    *input = rest;
    Ok(u16::from_be_bytes(x.try_into().unwrap()))
}
fn read_u32(input: &mut &[u8]) -> Result<u32> {
    if input.len() < 4 {
        bail!("missing u32")
    };
    let (x, rest) = input.split_at(4);
    *input = rest;
    Ok(u32::from_be_bytes(x.try_into().unwrap()))
}
fn read_u64(input: &mut &[u8]) -> Result<u64> {
    if input.len() < 8 {
        bail!("missing u64")
    };
    let (x, rest) = input.split_at(8);
    *input = rest;
    Ok(u64::from_be_bytes(x.try_into().unwrap()))
}
fn render_translation(key: &str, args: &str) -> String {
    if key == "chat.type.text" {
        format!("<{args}>")
    } else if args.is_empty() {
        key.to_owned()
    } else {
        format!("{key} {args}")
    }
}

pub fn normalize_chat(raw: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        return normalize_value(value);
    }
    let mut output = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            let _ = chars.next();
        } else {
            output.push(ch);
        }
    }
    output.replace("\\n", "\n").trim().to_owned()
}

fn normalize_value(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value,
        serde_json::Value::Array(values) => values.into_iter().map(normalize_value).collect(),
        serde_json::Value::Object(object) => {
            let mut text = object
                .get("text")
                .cloned()
                .map(normalize_value)
                .unwrap_or_default();
            if let Some(translate) = object.get("translate").and_then(serde_json::Value::as_str) {
                text.push_str(translate);
            }
            if let Some(extra) = object.get("extra") {
                text.push_str(&normalize_value(extra.clone()));
            }
            text
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{net::TcpListener, time::timeout};

    #[test]
    fn wire_varints_are_bounded_and_round_trip() {
        for value in [0, 1, 127, 128, 16_383, 16_384, i32::MAX, -1] {
            let mut bytes = Vec::new();
            write_varint(value, &mut bytes);
            let mut input = bytes.as_slice();
            assert_eq!(read_varint(&mut input).unwrap(), value);
            assert!(input.is_empty());
        }
        let mut bad = &[0x80, 0x80, 0x80, 0x80, 0x80][..];
        assert!(read_varint(&mut bad).is_err());
    }

    #[test]
    fn cfb8_matches_aes_stream_and_keeps_state_across_chunks() {
        let key = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut plaintext = [0u8; 32];
        for (i, byte) in plaintext.iter_mut().enumerate() {
            *byte = (i * 17) as u8;
        }
        let mut one = Cfb8::new(&key);
        let mut ciphertext = plaintext;
        one.encrypt(&mut ciphertext);
        let mut split = Cfb8::new(&key);
        let mut chunked = plaintext;
        split.encrypt(&mut chunked[..7]);
        split.encrypt(&mut chunked[7..]);
        assert_eq!(ciphertext, chunked);
        let mut decrypt = Cfb8::new(&key);
        decrypt.decrypt(&mut ciphertext);
        assert_eq!(ciphertext, plaintext);
    }

    #[test]
    fn network_nbt_reads_nested_chat_component() {
        let mut bytes = vec![10];
        bytes.extend_from_slice(&[
            8, 0, 4, b't', b'e', b'x', b't', 0, 5, b'h', b'e', b'l', b'l', b'o',
        ]);
        bytes.extend_from_slice(&[0]);
        let mut input = bytes.as_slice();
        let tag = parse_network_nbt(&mut input).unwrap();
        assert_eq!(tag.to_plain(), "hello");
        assert!(input.is_empty());
    }

    #[test]
    fn exact_palette_rejects_guessing() {
        assert!(Palette::for_protocol(774).is_ok());
        assert!(Palette::for_protocol(766).is_err());
        assert_eq!(
            Palette::for_protocol(774).unwrap().play_ids().system_chat,
            0x77
        );
    }

    #[tokio::test]
    async fn login_configuration_play_state_transition_and_keepalive() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _handshake = read_raw_frame(&mut socket).await.unwrap();
            let _login = read_raw_frame(&mut socket).await.unwrap();
            let mut login_success = vec![0x02];
            login_success.extend_from_slice(&[0; 16]);
            login_success.push(0);
            write_raw_frame(&mut socket, &login_success).await.unwrap();
            let _login_ack = read_raw_frame(&mut socket).await.unwrap();
            let _client_info = read_raw_frame(&mut socket).await.unwrap();
            write_raw_frame(&mut socket, &[0x03]).await.unwrap();
            let _finish_ack = read_raw_frame(&mut socket).await.unwrap();
            write_raw_frame(&mut socket, &[0x2b, 0, 0, 0, 0, 0, 0, 0, 7])
                .await
                .unwrap();
            let response = read_raw_frame(&mut socket).await.unwrap();
            assert_eq!(response[0], 0x1b);
        });
        let session = connect(
            "127.0.0.1",
            port,
            774,
            "tester",
            "00000000-0000-0000-0000-000000000001",
            "",
        )
        .await
        .unwrap();
        let mut state = session.state.clone();
        timeout(Duration::from_secs(2), async {
            while *state.borrow() != ConnectionState::Connected {
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(session.is_connected());
        // Let the server observe the PLAY keep-alive response before asking the actor to close.
        server.await.unwrap();
        session.disconnect().await.unwrap();
    }
}
