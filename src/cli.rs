use crate::{
    app::AppState,
    auth,
    domain::{TradeAction, TradeRequest},
    qq,
};
use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use std::sync::atomic::Ordering;
#[derive(Parser, Debug)]
#[command(name = "bilicraft-ai-stock", version)]
pub struct Cli {
    #[arg(long, default_value = ".")]
    pub root: std::path::PathBuf,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    Login,
    Serve,
    Status,
    Connect,
    Disconnect,
    Market {
        #[command(subcommand)]
        command: MarketCommand,
    },
    Portfolio {
        #[command(subcommand)]
        command: PortfolioCommand,
    },
    Watchlist {
        #[command(subcommand)]
        command: WatchCommand,
    },
    Trade {
        #[command(subcommand)]
        command: TradeCommand,
    },
    Ai {
        #[command(subcommand)]
        command: AiCommand,
    },
    Risk {
        #[command(subcommand)]
        command: RiskCommand,
    },
    Qq {
        #[command(subcommand)]
        command: QqCommand,
    },
    Audit,
    Init,
}
#[derive(Subcommand, Debug)]
pub enum MarketCommand {
    List,
    Health,
    Kline { company_id: i64 },
}
#[derive(Subcommand, Debug)]
pub enum PortfolioCommand {
    Show,
    Refresh,
    History,
}
#[derive(Subcommand, Debug)]
pub enum WatchCommand {
    List,
    Add { company_id: i64 },
    Remove { company_id: i64 },
}
#[derive(Subcommand, Debug)]
pub enum TradeCommand {
    Buy { company_id: i64, shares: i64 },
    Sell { company_id: i64, shares: i64 },
}
#[derive(Subcommand, Debug)]
pub enum AiCommand {
    Status,
    RunOnce,
    Ask { question: String },
    Pause,
    Resume,
}
#[derive(Subcommand, Debug)]
pub enum RiskCommand {
    Show,
    Validate,
}
#[derive(Subcommand, Debug)]
pub enum QqCommand {
    Setup,
    Status,
    Test,
}

pub async fn run(cli: Cli) -> Result<()> {
    let root = cli.root.canonicalize().unwrap_or(cli.root.clone());
    let state = AppState::load(&root)?;
    match cli.command {
        Command::Init => {
            state.config.save(&root.join("config.toml"))?;
            println!("已生成 config.toml");
        }
        Command::Serve => crate::server::serve(state).await?,
        Command::Status => println!("{}", serde_json::to_string_pretty(&state.status())?),
        Command::Connect => {
            let response = state
                .client
                .post(format!("http://{}/api/connect", state.config.web.bind))
                .send()
                .await?;
            println!("{}", response.text().await?);
        }
        Command::Disconnect => {
            let response = state
                .client
                .post(format!("http://{}/api/disconnect", state.config.web.bind))
                .send()
                .await?;
            println!("{}", response.text().await?);
        }
        Command::Login => {
            let mut sec = state.secrets.write().clone();
            let session =
                auth::device_login(&state.client, &state.config.paths.secrets_file, &mut sec)
                    .await?;
            *state.secrets.write() = sec;
            println!("登录成功：{} ({})", session.name, session.uuid);
        }
        Command::Market { command } => match command {
            MarketCommand::List => println!(
                "{}",
                serde_json::to_string_pretty(&state.market.companies().await?)?
            ),
            MarketCommand::Health => println!(
                "{}",
                serde_json::to_string_pretty(&state.market.health().await?)?
            ),
            MarketCommand::Kline { company_id } => println!(
                "{}",
                serde_json::to_string_pretty(
                    &state
                        .market
                        .kline(company_id, &state.config.market.interval)
                        .await?
                )?
            ),
        },
        Command::Portfolio { command } => match command {
            PortfolioCommand::Show | PortfolioCommand::Refresh => {
                print_server_json(&state, "/api/portfolio", "GET").await?
            }
            PortfolioCommand::History => println!(
                "{}",
                serde_json::to_string_pretty(&state.db.lock().profit_history(365)?)?
            ),
        },
        Command::Watchlist { command } => match command {
            WatchCommand::List => println!(
                "{}",
                serde_json::to_string_pretty(&state.db.lock().watchlist()?)?
            ),
            WatchCommand::Add { company_id } => {
                state.db.lock().set_watch(company_id, true)?;
            }
            WatchCommand::Remove { company_id } => {
                state.db.lock().set_watch(company_id, false)?;
            }
        },
        Command::Trade { command } => {
            let (action, company_id, shares) = match command {
                TradeCommand::Buy { company_id, shares } => (TradeAction::Buy, company_id, shares),
                TradeCommand::Sell { company_id, shares } => {
                    (TradeAction::Sell, company_id, shares)
                }
            };
            let response = state
                .client
                .post(format!("http://{}/api/trade", state.config.web.bind))
                .json(&TradeRequest {
                    action,
                    company_id,
                    shares,
                    source: "manual-cli".into(),
                    reason: None,
                })
                .send()
                .await?;
            println!("{}", response.text().await?);
        }
        Command::Ai { command } => match command {
            AiCommand::Status => println!("{}", serde_json::to_string_pretty(&state.status())?),
            AiCommand::RunOnce => post_ai(&state, None).await?,
            AiCommand::Ask { question } => post_ai(&state, Some(question)).await?,
            AiCommand::Pause => post_server(&state, "/api/ai/pause").await?,
            AiCommand::Resume => post_server(&state, "/api/ai/resume").await?,
        },
        Command::Risk { command } => match command {
            RiskCommand::Show => println!("{}", toml::to_string_pretty(&state.config.risk)?),
            RiskCommand::Validate => {
                let errors = validate_risk(&state);
                if errors.is_empty() {
                    println!("risk configuration is valid");
                } else {
                    return Err(anyhow!(errors.join("; ")));
                }
            }
        },
        Command::Qq { command } => match command {
            QqCommand::Setup => {
                let mut sec = state.secrets.write().clone();
                qq::setup(&state.config.paths.secrets_file, &mut sec).await?;
                *state.secrets.write() = sec;
            }
            QqCommand::Status => {
                println!("configured={}", state.secrets.read().qq_app_id.is_some())
            }
            QqCommand::Test => println!("QQ test requires configured targets"),
        },
        Command::Audit => println!(
            "{}",
            serde_json::to_string_pretty(&state.db.lock().audits(100)?)?
        ),
    }
    Ok(())
}
async fn print_server_json(state: &AppState, path: &str, _method: &str) -> Result<()> {
    let response = state
        .client
        .get(format!("http://{}{}", state.config.web.bind, path))
        .send()
        .await?;
    println!("{}", response.text().await?);
    Ok(())
}
async fn post_server(state: &AppState, path: &str) -> Result<()> {
    let response = state
        .client
        .post(format!("http://{}{}", state.config.web.bind, path))
        .send()
        .await?;
    println!("{}", response.text().await?);
    Ok(())
}
async fn post_ai(state: &AppState, question: Option<String>) -> Result<()> {
    let response = state
        .client
        .post(format!("http://{}/api/ai/run", state.config.web.bind))
        .json(&serde_json::json!({"question":question}))
        .send()
        .await?;
    println!("{}", response.text().await?);
    Ok(())
}
fn validate_risk(state: &AppState) -> Vec<String> {
    let r = &state.config.risk;
    let mut errors = Vec::new();
    if r.ai_budget < 0.0 || r.ai_budget_percent < 0.0 {
        errors.push("AI budget cannot be negative".into())
    }
    if r.max_single_trade < 0.0 || r.max_daily_loss < 0.0 {
        errors.push("trade and loss limits cannot be negative".into())
    }
    if state.config.risk.automation_enabled
        && (!state.config.ai.enabled || r.ai_budget <= 0.0 && r.ai_budget_percent <= 0.0)
    {
        errors.push("automation requires AI enabled and a non-zero budget".into())
    }
    errors
}
