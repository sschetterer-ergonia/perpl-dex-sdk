//! Margin top-up bot for Perpl DEX.
//!
//! This binary monitors positions and automatically tops up collateral
//! when leverage exceeds a configured threshold.

mod bot;
mod config;
mod error;
mod margin;

use alloy::{network::EthereumWallet, primitives::Address, signers::local::PrivateKeySigner};
use clap::Parser;
use dex_sdk::Chain;
use std::{process::exit, time::Duration};
use tracing::error;
use url::Url;

use bot::MarginTopUpBot;
use config::{CliConfig, EnvConfig};

#[tokio::main]
async fn main() {
    // Load .env file
    if let Err(e) = dotenvy::dotenv() {
        eprintln!("Warning: Failed to load .env file: {}", e);
    }

    // Parse environment configuration
    let env_config = match EnvConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to parse environment configuration: {}", e);
            exit(1);
        }
    };

    // Parse CLI arguments
    let cli_config = CliConfig::parse();

    // Convert to strategy config
    let topup_config = match cli_config.to_topup_config() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Invalid configuration: {}", e);
            exit(1);
        }
    };

    // Set up logging
    if std::env::var("RUST_LOG").is_err() {
        unsafe {
            std::env::set_var("RUST_LOG", "info");
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Parse addresses
    let collateral_token_address: Address = match env_config.collateral_token_address() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("Invalid collateral token address: {}", e);
            exit(1);
        }
    };

    let exchange_address: Address = match env_config.exchange_address() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("Invalid exchange address: {}", e);
            exit(1);
        }
    };

    // Parse private key
    let private_key: PrivateKeySigner = match env_config.private_key.parse() {
        Ok(key) => key,
        Err(e) => {
            eprintln!("Invalid private key: {}", e);
            exit(1);
        }
    };

    let wallet = EthereumWallet::new(private_key);

    // Parse RPC URL
    let node_url = match Url::parse(&env_config.node_rpc_url) {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Invalid RPC URL: {}", e);
            exit(1);
        }
    };

    // Get perpetual IDs from config or use default
    let perpetual_ids = if topup_config.perpetual_ids.is_empty() {
        // If no specific perpetuals in CLI, could add a default here
        vec![]
    } else {
        topup_config
            .perpetual_ids
            .iter()
            .map(|id| u32::from(*id))
            .collect()
    };

    // Create chain configuration
    let chain = Chain::custom(
        env_config.chain_id,
        collateral_token_address,
        env_config.deployed_at_block,
        exchange_address,
        perpetual_ids,
    );

    // Default timeout is 30 seconds
    let timeout = Duration::from_secs(env_config.timeout_seconds.unwrap_or(30));

    // Create and run the bot
    let mut bot = match MarginTopUpBot::try_new(node_url, wallet, chain, topup_config, timeout).await
    {
        Ok(bot) => bot,
        Err(e) => {
            eprintln!("Failed to create margin top-up bot: {}", e);
            exit(1);
        }
    };

    if let Err(e) = bot.run().await {
        error!(%e, "Margin top-up bot encountered an error, shutting down");
        exit(1);
    }
}
