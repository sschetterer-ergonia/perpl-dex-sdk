//! L3 order representation.

use crate::{state::Order, types};
use fastnum::UD64;

/// Key for L3 order time-priority ordering within a price level.
/// Tuple of (block_number, order_id) provides FIFO ordering.
pub type L3OrderKey = (u64, types::OrderId);

/// Individual order in the L3 book.
#[derive(Clone, Debug)]
pub struct L3Order {
    order: Order,
}

impl L3Order {
    /// Create a new L3 order.
    pub fn new(order: Order) -> Self {
        Self { order }
    }

    /// The underlying order.
    pub fn order(&self) -> &Order {
        &self.order
    }

    /// Account ID that placed this order.
    pub fn account_id(&self) -> types::AccountId {
        self.order.account_id()
    }

    /// Order ID.
    pub fn order_id(&self) -> types::OrderId {
        self.order.order_id()
    }

    /// Order size.
    pub fn size(&self) -> UD64 {
        self.order.size()
    }

    /// Order price.
    pub fn price(&self) -> UD64 {
        self.order.price()
    }

    /// Order type.
    pub fn r#type(&self) -> types::OrderType {
        self.order.r#type()
    }

    /// The L3 ordering key for this order.
    pub(crate) fn key(&self) -> L3OrderKey {
        (self.order.instant().block_number(), self.order.order_id())
    }

    /// Update the underlying order (for size changes).
    pub(crate) fn update_order(&mut self, order: Order) {
        self.order = order;
    }
}
