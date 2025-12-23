//! Configuration for the margin top-up bot.
//!
//! Configuration comes from two sources:
//! - Environment variables (via .env file or shell): connection details, keys
//! - CLI arguments: strategy parameters

use alloy::primitives::Address;
use clap::Parser;
use dex_sdk::types::PerpetualId;
use fastnum::{UD64, UD128, decimal::Context};

use crate::margin::TopUpConfig;

/// Environment configuration (connection details, credentials).
#[derive(Debug, serde::Deserialize)]
pub struct EnvConfig {
    /// Chain ID (e.g., 421614 for Arbitrum Sepolia)
    pub chain_id: u64,

    /// Collateral token address
    pub collateral_token_address: String,

    /// Exchange contract address
    pub address: String,

    /// Private key for signing transactions
    pub private_key: String,

    /// Block number when the exchange was deployed
    pub deployed_at_block: u64,

    /// RPC URL for the node
    pub node_rpc_url: String,

    /// Optional timeout for operations (default: 30s)
    pub timeout_seconds: Option<u64>,
}

impl EnvConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::from_env()
    }

    /// Parse the collateral token address.
    pub fn collateral_token_address(&self) -> Result<Address, alloy::primitives::hex::FromHexError> {
        self.collateral_token_address.parse()
    }

    /// Parse the exchange address.
    pub fn exchange_address(&self) -> Result<Address, alloy::primitives::hex::FromHexError> {
        self.address.parse()
    }
}

/// CLI arguments for the margin top-up strategy.
#[derive(Debug, Parser)]
#[command(name = "margin-topup")]
#[command(about = "Margin top-up bot for Perpl DEX positions")]
pub struct CliConfig {
    /// Leverage threshold that triggers a top-up (e.g., 15.0)
    #[arg(long, default_value = "15")]
    pub trigger_leverage: String,

    /// Target leverage after top-up (e.g., 10.0)
    #[arg(long, default_value = "10")]
    pub target_leverage: String,

    /// Perpetual IDs to monitor (comma-separated, e.g., "1,2,3")
    /// If not specified, monitors all perpetuals
    #[arg(long, value_delimiter = ',')]
    pub perpetual_ids: Vec<u32>,

    /// Minimum balance to keep in reserve (not used for top-ups)
    #[arg(long, default_value = "0")]
    pub min_reserve_balance: String,
}

impl CliConfig {
    /// Convert CLI config to the pure TopUpConfig used by the strategy.
    pub fn to_topup_config(&self) -> Result<TopUpConfig, ConfigError> {
        let trigger_leverage = UD64::from_str(&self.trigger_leverage, Context::default())
            .map_err(|_| ConfigError::InvalidLeverage("trigger_leverage".to_string()))?;

        let target_leverage = UD64::from_str(&self.target_leverage, Context::default())
            .map_err(|_| ConfigError::InvalidLeverage("target_leverage".to_string()))?;

        if target_leverage >= trigger_leverage {
            return Err(ConfigError::InvalidLeverageRelation);
        }

        if target_leverage == UD64::ZERO {
            return Err(ConfigError::ZeroTargetLeverage);
        }

        let min_reserve_balance = UD128::from_str(&self.min_reserve_balance, Context::default())
            .map_err(|_| ConfigError::InvalidReserveBalance)?;

        let perpetual_ids: Vec<PerpetualId> = self
            .perpetual_ids
            .iter()
            .map(|&id| PerpetualId::from(id))
            .collect();

        Ok(TopUpConfig {
            trigger_leverage,
            target_leverage,
            perpetual_ids,
            min_reserve_balance,
        })
    }
}

/// Configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Invalid leverage value for {0}")]
    InvalidLeverage(String),

    #[error("target_leverage must be less than trigger_leverage")]
    InvalidLeverageRelation,

    #[error("target_leverage cannot be zero")]
    ZeroTargetLeverage,

    #[error("Invalid reserve balance value")]
    InvalidReserveBalance,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_config_to_topup_config() {
        let cli = CliConfig {
            trigger_leverage: "15".to_string(),
            target_leverage: "10".to_string(),
            perpetual_ids: vec![1, 2],
            min_reserve_balance: "100".to_string(),
        };

        let config = cli.to_topup_config().unwrap();
        assert_eq!(config.trigger_leverage, UD64::from_str("15", Context::default()).unwrap());
        assert_eq!(config.target_leverage, UD64::from_str("10", Context::default()).unwrap());
        assert_eq!(config.perpetual_ids.len(), 2);
    }

    #[test]
    fn test_invalid_leverage_relation() {
        let cli = CliConfig {
            trigger_leverage: "10".to_string(),
            target_leverage: "15".to_string(),
            perpetual_ids: vec![],
            min_reserve_balance: "0".to_string(),
        };

        assert!(matches!(
            cli.to_topup_config(),
            Err(ConfigError::InvalidLeverageRelation)
        ));
    }

    #[test]
    fn test_zero_target_leverage() {
        let cli = CliConfig {
            trigger_leverage: "15".to_string(),
            target_leverage: "0".to_string(),
            perpetual_ids: vec![],
            min_reserve_balance: "0".to_string(),
        };

        assert!(matches!(
            cli.to_topup_config(),
            Err(ConfigError::ZeroTargetLeverage)
        ));
    }
}
