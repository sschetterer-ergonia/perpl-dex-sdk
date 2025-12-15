//! Fill data structures.

use alloy::primitives::TxHash;
use fastnum::UD64;
use tokio::sync::mpsc;

use crate::types;

/// A matched trade between a taker and maker.
///
/// Each trade represents a single match where a taker order
/// was executed against a maker order at a specific price and size.
#[derive(Clone, Debug)]
pub struct Trade {
    /// Transaction hash the trade occurred in.
    pub tx_hash: TxHash,

    /// Transaction index within the block.
    pub tx_index: u64,

    /// Log index of the maker fill event.
    pub log_index: u64,

    /// Perpetual contract ID.
    pub perpetual_id: types::PerpetualId,

    /// Fill price (normalized decimal).
    pub price: UD64,

    /// Fill size (normalized decimal).
    pub size: UD64,

    /// Maker account ID.
    pub maker_account_id: types::AccountId,

    /// Maker order ID.
    pub maker_order_id: types::OrderId,

    /// Maker fee paid (normalized decimal, in collateral token).
    pub maker_fee: UD64,

    /// Taker account ID.
    pub taker_account_id: types::AccountId,

    /// Taker fee paid (normalized decimal, in collateral token).
    pub taker_fee: UD64,
}

/// Trades from a single block.
#[derive(Clone, Debug)]
pub struct BlockTrades {
    /// Block instant.
    pub instant: types::StateInstant,

    /// All trades in this block.
    pub trades: Vec<Trade>,
}

impl BlockTrades {
    pub(crate) fn new(instant: types::StateInstant, trades: Vec<Trade>) -> Self {
        Self { instant, trades }
    }

    /// Returns true if there are no trades in this block.
    pub fn is_empty(&self) -> bool {
        self.trades.is_empty()
    }

    /// Returns the number of trades in this block.
    pub fn len(&self) -> usize {
        self.trades.len()
    }
}

/// Receiver for block trades.
pub struct TradeReceiver {
    inner: mpsc::Receiver<BlockTrades>,
}

impl TradeReceiver {
    pub(crate) fn new(inner: mpsc::Receiver<BlockTrades>) -> Self {
        Self { inner }
    }

    /// Receives the next batch of trades, or `None` if the channel is closed.
    pub async fn recv(&mut self) -> Option<BlockTrades> {
        self.inner.recv().await
    }
}
