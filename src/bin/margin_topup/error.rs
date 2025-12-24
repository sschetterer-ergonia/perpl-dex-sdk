//! Error types for the margin top-up bot.

use dex_sdk::{error::DexError, types::PerpetualId};

use crate::config::ConfigError;

/// Main error type for the margin top-up bot.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Environment configuration error: {0}")]
    EnvConfig(#[from] envy::Error),

    #[error("Alloy contract error: {0}")]
    AlloyContract(#[from] alloy::contract::Error),

    #[error("Alloy signer error: {0}")]
    AlloySigner(#[from] alloy::signers::local::LocalSignerError),

    #[error("Alloy pending transaction error: {0}")]
    AlloyPendingTransaction(#[from] alloy::providers::PendingTransactionError),

    #[error("DEX SDK error: {0}")]
    Dex(#[from] DexError),

    #[error("Invalid RPC URL: {0}")]
    InvalidRpcUrl(#[from] url::ParseError),

    #[error("Invalid address: {0}")]
    InvalidAddress(#[from] alloy::primitives::hex::FromHexError),

    #[error("No account found for wallet address")]
    NoAccountFound,

    #[error("Perpetual {0} not found in exchange state")]
    PerpetualNotFound(PerpetualId),

    #[error("Position not found for perpetual {0}")]
    PositionNotFound(PerpetualId),

    #[error("Transaction timeout after {0} seconds")]
    TransactionTimeout(u64),

    #[error("Event stream closed unexpectedly")]
    StreamClosed,

    #[error("Event stream error")]
    StreamError,
}

pub type Result<T> = std::result::Result<T, Error>;
