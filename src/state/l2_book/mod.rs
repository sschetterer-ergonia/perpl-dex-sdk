//! L2/L3 order book implementation.
//!
//! This module provides the order book data structure that tracks orders
//! at each price level with FIFO time-priority ordering.

mod level;
mod order;

#[cfg(test)]
mod tests;

pub use level::L3Level;
pub use order::{L3Order, L3OrderKey};

use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, btree_map},
};

use fastnum::{UD64, UD128};
use itertools::{FoldWhile, Itertools};

use crate::{state::Order, types};

/// BTreeMap-based L2/L3 order book.
///
/// Tracks individual orders per price level in time-priority order (L3),
/// while maintaining aggregated L2 statistics via cached values.
#[derive(Clone, Debug, Default)]
pub struct L2Book {
    asks: BTreeMap<UD64, L3Level>,
    bids: BTreeMap<Reverse<UD64>, L3Level>,
    /// Reverse index: order_id -> (price, block_number) for O(1) lookups.
    order_index: HashMap<types::OrderId, (UD64, u64)>,
}

impl L2Book {
    pub(crate) fn new() -> Self {
        Self {
            asks: BTreeMap::new(),
            bids: BTreeMap::new(),
            order_index: HashMap::new(),
        }
    }

    // === L2 API (backwards compatible) ===

    /// Asks sorted away from the spread.
    pub fn asks(&self) -> &BTreeMap<UD64, L3Level> {
        &self.asks
    }

    /// Bids sorted away from the spread.
    pub fn bids(&self) -> &BTreeMap<Reverse<UD64>, L3Level> {
        &self.bids
    }

    /// Best ask price/size.
    pub fn best_ask(&self) -> Option<(UD64, UD64)> {
        self.asks
            .first_key_value()
            .map(|(k, v)| (*k, v.size()))
    }

    /// Best bid price/size.
    pub fn best_bid(&self) -> Option<(UD64, UD64)> {
        self.bids
            .first_key_value()
            .map(|(k, v)| (k.0, v.size()))
    }

    /// Ask impact price for the requested size, along with the fillable size and size-averaged price.
    pub fn ask_impact(&self, want_size: UD64) -> Option<(UD64, UD64, UD64)> {
        Self::impact(self.asks.iter(), want_size)
    }

    /// Bid impact price for the requested size, along with the fillable size and size-averaged price.
    pub fn bid_impact(&self, want_size: UD64) -> Option<(UD64, UD64, UD64)> {
        Self::impact(self.bids.iter().map(|(k, v)| (&k.0, v)), want_size)
    }

    // === L3 API (new) ===

    /// Get L3 level at a specific ask price.
    pub fn ask_level(&self, price: UD64) -> Option<&L3Level> {
        self.asks.get(&price)
    }

    /// Get L3 level at a specific bid price.
    pub fn bid_level(&self, price: UD64) -> Option<&L3Level> {
        self.bids.get(&Reverse(price))
    }

    /// Get a specific order by ID (O(1) via reverse index).
    pub fn get_order(&self, order_id: types::OrderId) -> Option<&L3Order> {
        let (price, block_number) = self.order_index.get(&order_id)?;
        let key = (*block_number, order_id);
        // Determine side by checking both - order could be on either
        if let Some(level) = self.asks.get(price) {
            if let Some(order) = level.get_order(&key) {
                return Some(order);
            }
        }
        if let Some(level) = self.bids.get(&Reverse(*price)) {
            if let Some(order) = level.get_order(&key) {
                return Some(order);
            }
        }
        None
    }

    /// Iterator over all L3 orders on the ask side in price-time priority.
    pub fn ask_orders(&self) -> impl Iterator<Item = &L3Order> {
        self.asks.values().flat_map(|level| level.orders())
    }

    /// Iterator over all L3 orders on the bid side in price-time priority.
    pub fn bid_orders(&self) -> impl Iterator<Item = &L3Order> {
        self.bids.values().flat_map(|level| level.orders())
    }

    /// Total number of orders in the book.
    pub fn total_orders(&self) -> usize {
        self.order_index.len()
    }

    // === Mutation methods ===

    /// Add an order to the book.
    pub(crate) fn add_order(&mut self, order: &Order) {
        let l3_order = L3Order::new(*order);
        let key = l3_order.key();

        // Update reverse index
        self.order_index
            .insert(order.order_id(), (order.price(), key.0));

        match order.r#type().side() {
            types::OrderSide::Ask => match self.asks.entry(order.price()) {
                btree_map::Entry::Vacant(v) => {
                    let mut level = L3Level::new();
                    level.add_order(l3_order);
                    v.insert(level);
                }
                btree_map::Entry::Occupied(mut o) => {
                    o.get_mut().add_order(l3_order);
                }
            },
            types::OrderSide::Bid => match self.bids.entry(Reverse(order.price())) {
                btree_map::Entry::Vacant(v) => {
                    let mut level = L3Level::new();
                    level.add_order(l3_order);
                    v.insert(level);
                }
                btree_map::Entry::Occupied(mut o) => {
                    o.get_mut().add_order(l3_order);
                }
            },
        }
    }

    /// Update an order's size (same price level).
    pub(crate) fn update_order(&mut self, order: &Order, _prev_order: &Order) {
        // Look up the order's position via the index
        let Some(&(indexed_price, block_number)) = self.order_index.get(&order.order_id()) else {
            return; // Order not found
        };

        let key = (block_number, order.order_id());

        match order.r#type().side() {
            types::OrderSide::Ask => {
                if let Some(level) = self.asks.get_mut(&indexed_price) {
                    level.update_order(&key, *order);
                }
            }
            types::OrderSide::Bid => {
                if let Some(level) = self.bids.get_mut(&Reverse(indexed_price)) {
                    level.update_order(&key, *order);
                }
            }
        }
    }

    /// Remove an order from the book.
    pub(crate) fn remove_order(&mut self, order: &Order) {
        // Look up and remove from index
        let Some((indexed_price, block_number)) = self.order_index.remove(&order.order_id()) else {
            return; // Order not found
        };

        let key = (block_number, order.order_id());

        match order.r#type().side() {
            types::OrderSide::Ask => {
                if let btree_map::Entry::Occupied(mut o) = self.asks.entry(indexed_price) {
                    o.get_mut().remove_order(&key);
                    if o.get().is_empty() {
                        o.remove();
                    }
                }
            }
            types::OrderSide::Bid => {
                if let btree_map::Entry::Occupied(mut o) = self.bids.entry(Reverse(indexed_price)) {
                    o.get_mut().remove_order(&key);
                    if o.get().is_empty() {
                        o.remove();
                    }
                }
            }
        }
    }

    fn impact<'a>(
        mut side: impl Iterator<Item = (&'a UD64, &'a L3Level)>,
        want_size: UD64,
    ) -> Option<(UD64, UD64, UD64)> {
        let (price, unfilled, price_size) = side
            .fold_while(
                (UD64::ZERO, want_size, UD128::ZERO),
                |(_, unfilled, price_size), (price, level)| {
                    let level_size = level.size();
                    if unfilled > level_size {
                        FoldWhile::Continue((
                            *price,
                            unfilled - level_size,
                            price_size + (price.resize() * level_size.resize()),
                        ))
                    } else {
                        FoldWhile::Done((
                            *price,
                            UD64::ZERO,
                            price_size + (price.resize() * unfilled.resize()),
                        ))
                    }
                },
            )
            .into_inner();
        let filled = want_size - unfilled;
        if filled > UD64::ZERO {
            Some((price, filled, (price_size / filled.resize()).resize()))
        } else {
            None
        }
    }
}
