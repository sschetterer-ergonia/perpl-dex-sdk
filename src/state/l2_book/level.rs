//! L3 price level containing individual orders.

use std::collections::BTreeMap;

use fastnum::UD64;

use super::order::{L3Order, L3OrderKey};
use crate::state::Order;

/// L3 price level containing individual orders in time-priority order.
/// Named L3Level but used internally - the book is still called L2Book for backwards compatibility.
#[derive(Clone, Debug, Default)]
pub struct L3Level {
    /// Individual orders keyed by (block_number, order_id) for FIFO ordering.
    orders: BTreeMap<L3OrderKey, L3Order>,
    /// Cached aggregate: total size at this level.
    cached_size: UD64,
    /// Cached aggregate: number of orders at this level.
    cached_num_orders: u32,
}

impl L3Level {
    /// Create a new empty L3 level.
    pub fn new() -> Self {
        Self::default()
    }

    /// Total size at this price level (cached, O(1)).
    pub fn size(&self) -> UD64 {
        self.cached_size
    }

    /// Number of orders at this price level (cached, O(1)).
    pub fn num_orders(&self) -> u32 {
        self.cached_num_orders
    }

    /// Iterator over individual orders in FIFO order.
    pub fn orders(&self) -> impl Iterator<Item = &L3Order> {
        self.orders.values()
    }

    /// Get a specific order by its key.
    pub fn get_order(&self, key: &L3OrderKey) -> Option<&L3Order> {
        self.orders.get(key)
    }

    /// First (oldest) order at this level.
    pub fn first_order(&self) -> Option<&L3Order> {
        self.orders.first_key_value().map(|(_, v)| v)
    }

    /// Add an order to this level.
    pub(crate) fn add_order(&mut self, l3_order: L3Order) {
        self.cached_size += l3_order.size();
        self.cached_num_orders += 1;
        self.orders.insert(l3_order.key(), l3_order);
    }

    /// Update an order's size at this level. Returns the previous size if found.
    pub(crate) fn update_order(&mut self, key: &L3OrderKey, updated_order: Order) -> Option<UD64> {
        if let Some(l3_order) = self.orders.get_mut(key) {
            let prev_size = l3_order.size();
            self.cached_size -= prev_size;
            self.cached_size += updated_order.size();
            l3_order.update_order(updated_order);
            Some(prev_size)
        } else {
            None
        }
    }

    /// Remove an order from this level. Returns the removed order if found.
    pub(crate) fn remove_order(&mut self, key: &L3OrderKey) -> Option<L3Order> {
        if let Some(l3_order) = self.orders.remove(key) {
            self.cached_size -= l3_order.size();
            self.cached_num_orders -= 1;
            Some(l3_order)
        } else {
            None
        }
    }

    /// Check if this level has no orders.
    pub(crate) fn is_empty(&self) -> bool {
        self.cached_num_orders == 0
    }
}
