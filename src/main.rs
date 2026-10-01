use clap::Parser;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();
    bilicraft_ai_stock::cli::run(bilicraft_ai_stock::cli::Cli::parse()).await
}
