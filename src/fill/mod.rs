//! Trade listener module for streaming normalized trade events.
//!
//! Listens to `MakerOrderFilled` and `TakerOrderFilled` events, matches them
//! into unified `Trade` events, normalizes fixed-point values to decimals,
//! and streams trades batched per block.
//!
//! # Architecture
//!
//! The module separates pure processing logic from async I/O:
//!
//! - [`TradeProcessor`] - Pure, synchronous trade extraction from raw events
//! - [`NormalizationConfig`] - Configuration fetched once at startup
//! - [`start`] - Async entry point that spawns a background listener task
//!
//! # Example
//!
//! ```ignore
//! use dex_sdk::{Chain, fill, types::StateInstant};
//!
//! let chain = Chain::testnet();
//! let provider = /* setup provider */;
//! let from = StateInstant::new(latest_block, timestamp);
//!
//! let (mut rx, handle) = fill::start(&chain, provider, from, tokio::time::sleep).await?;
//!
//! while let Some(block_trades) = rx.recv().await {
//!     println!("Block {}: {} trades",
//!         block_trades.instant.block_number(),
//!         block_trades.trades.len()
//!     );
//!
//!     for trade in &block_trades.trades {
//!         println!("  {} @ {} (maker={}, taker={})",
//!             trade.size, trade.price, trade.maker_account_id, trade.taker_account_id);
//!     }
//! }
//!
//! // Check for errors
//! handle.await??;
//! ```

mod listener;
mod types;

pub use listener::{start, NormalizationConfig, TradeProcessor};
pub use types::{BlockTrades, Trade, TradeReceiver};
