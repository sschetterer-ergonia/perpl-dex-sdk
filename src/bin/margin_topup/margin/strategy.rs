//! Pure functional core for margin top-up decisions.
//!
//! This module contains the main decision-making logic for the top-up bot.
//! All functions are pure - they take state as input and return decisions,
//! with NO side effects (no IO, no logging, no state mutation).
//!
//! Each account is processed independently - these functions operate on a
//! single account at a time.

use dex_sdk::state::{Account, Position};
use fastnum::{UD64, UD128};

use super::calc;
use super::types::{EvaluationSummary, PositionMarginInfo, TopUpAction, TopUpConfig};

/// Compute a single top-up action (or None) for an account.
///
/// This is the main entry point for the pure functional core.
///
/// Logic:
/// 1. Collect positions from the account
/// 2. Calculate leverage for each position
/// 3. Filter to over-leveraged positions (current_leverage > trigger_leverage)
/// 4. Sort by required top-up amount descending (largest need first)
/// 5. Return top-up for the position needing most capital
///
/// Note: We use a greedy approach - put all available capital into the position
/// that needs the most, even if it's not enough to reach target leverage.
/// Sorting by top-up amount (rather than leverage) prioritizes positions that
/// are furthest from their target in absolute capital terms.
///
/// This function does NO IO, NO logging - pure computation only.
pub fn compute_topup(account: &Account, config: &TopUpConfig) -> Option<TopUpAction> {
    let available_capital = calculate_available_capital(account, config);

    if available_capital == UD128::ZERO {
        return None;
    }

    // Collect over-leveraged positions with their leverage and required top-up
    let mut candidates: Vec<(&Position, UD64, UD128)> = account
        .positions()
        .values()
        .filter(|pos| {
            config.perpetual_ids.is_empty() || config.perpetual_ids.contains(&pos.perpetual_id())
        })
        .filter_map(|pos| {
            let leverage = calc::current_leverage(pos)?;
            if leverage <= config.trigger_leverage {
                return None;
            }
            let required = calc::required_topup_amount(pos, config.target_leverage)?;
            Some((pos, leverage, required))
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Sort by required top-up amount descending (largest need first)
    candidates.sort_by(|a, b| {
        b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Take the position needing most capital and top it up with whatever we have
    let (position, current_leverage, ideal_amount) = candidates[0];

    // Use min of ideal amount and available capital (partial top-up is fine)
    let amount = if ideal_amount <= available_capital {
        ideal_amount
    } else {
        available_capital
    };

    Some(TopUpAction {
        perpetual_id: position.perpetual_id(),
        amount,
        current_leverage,
        target_leverage: config.target_leverage,
    })
}

/// Compute a full evaluation summary for logging/diagnostics.
///
/// This provides detailed information about all positions, not just the one
/// we'll act on. Useful for logging and monitoring.
pub fn evaluate_all(account: &Account, config: &TopUpConfig) -> EvaluationSummary {
    let available_capital = calculate_available_capital(account, config);

    let mut position_infos: Vec<PositionMarginInfo> = Vec::new();
    let mut total_capital_needed = UD128::ZERO;

    for position in account.positions().values() {
        // Skip if not in monitored perpetuals (unless monitoring all)
        if !config.perpetual_ids.is_empty()
            && !config.perpetual_ids.contains(&position.perpetual_id())
        {
            continue;
        }

        let current_leverage = calc::current_leverage(position);
        let is_over_leveraged = current_leverage
            .map(|lev| lev > config.trigger_leverage)
            .unwrap_or(false);

        let required_topup = if is_over_leveraged {
            calc::required_topup_amount(position, config.target_leverage)
        } else {
            None
        };

        if let Some(amount) = required_topup {
            total_capital_needed += amount;
        }

        let can_topup = required_topup.is_some() && available_capital > UD128::ZERO;

        position_infos.push(PositionMarginInfo {
            perpetual_id: position.perpetual_id(),
            current_leverage,
            is_over_leveraged,
            required_topup,
            can_topup,
        });
    }

    let over_leveraged_count = position_infos.iter().filter(|p| p.is_over_leveraged).count();
    let positions_that_can_topup = position_infos.iter().filter(|p| p.can_topup).count();

    EvaluationSummary {
        positions_evaluated: position_infos.len(),
        over_leveraged_count,
        positions_that_can_topup,
        total_capital_needed,
        available_capital,
        position_infos,
    }
}

/// Calculate capital available for top-ups.
///
/// This is the account balance minus the reserve.
fn calculate_available_capital(account: &Account, config: &TopUpConfig) -> UD128 {
    let balance = account.balance();

    // Saturating subtraction - don't go below zero
    if balance > config.min_reserve_balance {
        balance - config.min_reserve_balance
    } else {
        UD128::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dex_sdk::testing::{AccountBuilder, PositionBuilder};
    use fastnum::{dec256, udec64, udec128};

    fn make_config(trigger: UD64, target: UD64) -> TopUpConfig {
        TopUpConfig {
            trigger_leverage: trigger,
            target_leverage: target,
            perpetual_ids: vec![],
            min_reserve_balance: UD128::ZERO,
        }
    }

    // ==================== Basic edge cases ====================

    #[test]
    fn test_calculate_available_capital_zero_balance() {
        let account = AccountBuilder::new().id(1).build();
        let config = make_config(udec64!(15), udec64!(10));

        let available = calculate_available_capital(&account, &config);
        assert_eq!(available, UD128::ZERO);
    }

    #[test]
    fn test_config_with_reserve_saturates_to_zero() {
        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(50))
            .build();
        let config = TopUpConfig {
            trigger_leverage: udec64!(15),
            target_leverage: udec64!(10),
            perpetual_ids: vec![],
            min_reserve_balance: udec128!(100),
        };

        let available = calculate_available_capital(&account, &config);
        assert_eq!(available, UD128::ZERO);
    }

    #[test]
    fn test_compute_topup_no_positions_returns_none() {
        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .build();
        let config = make_config(udec64!(15), udec64!(10));

        let action = compute_topup(&account, &config);
        assert!(action.is_none());
    }

    #[test]
    fn test_evaluate_all_no_positions_empty_summary() {
        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .build();
        let config = make_config(udec64!(15), udec64!(10));

        let summary = evaluate_all(&account, &config);
        assert_eq!(summary.positions_evaluated, 0);
        assert_eq!(summary.over_leveraged_count, 0);
        assert_eq!(summary.total_capital_needed, UD128::ZERO);
        assert_eq!(summary.available_capital, udec128!(500));
    }

    // ==================== Single position scenarios ====================

    #[test]
    fn test_single_position_under_threshold_no_action() {
        // Position at 5x leverage, threshold is 15x
        // notional = 100 * 10 = 1000, equity = 200, leverage = 5x
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(200))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .position(position)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        assert!(action.is_none());
    }

    #[test]
    fn test_single_position_over_threshold_returns_action() {
        // Position at 20x leverage, threshold is 15x
        // notional = 100 * 10 = 1000, equity = 50, leverage = 20x
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500)) // Enough capital
            .position(position)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        assert_eq!(action.perpetual_id, 1);
        // Target equity = 1000/10 = 100
        // Current equity = 50
        // Top-up = 100 - 50 = 50
        assert_eq!(action.amount, udec128!(50));
        assert_eq!(action.target_leverage, udec64!(10));
    }

    #[test]
    fn test_single_position_over_threshold_partial_topup() {
        // Position at 20x leverage needs 50 top-up, but only 30 available
        // Should still top up with 30 (partial is better than nothing)
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(30)) // Not enough for full top-up
            .position(position)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        // Should still return an action with partial amount
        assert!(action.is_some());
        let action = action.unwrap();
        assert_eq!(action.perpetual_id, 1);
        assert_eq!(action.amount, udec128!(30)); // All available capital
    }

    #[test]
    fn test_single_position_zero_capital_no_action() {
        // Position over threshold but zero capital available
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(UD128::ZERO) // No capital
            .position(position)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        assert!(action.is_none());
    }

    // ==================== Multiple positions - prioritization ====================

    #[test]
    fn test_multiple_positions_largest_topup_needed_first() {
        // Position 1: 20x leverage, needs 50 to reach 10x
        // notional = 1000, equity = 50, target_equity = 100, topup = 50
        let pos1 = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        // Position 2: 25x leverage, needs 60 to reach 10x
        // notional = 1000, equity = 40, target_equity = 100, topup = 60
        let pos2 = PositionBuilder::new()
            .perpetual_id(2)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(40))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .position(pos1)
            .position(pos2)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        // Position 2 needs more top-up (60 > 50), should be processed first
        assert_eq!(action.perpetual_id, 2);
        assert_eq!(action.amount, udec128!(60));
    }

    #[test]
    fn test_multiple_positions_greedy_into_largest_need() {
        // Position 1: Very leveraged (50x), needs 400 to reach 10x
        // notional = 5000, equity = 100, target_equity = 500, topup = 400
        let pos1 = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(500))
            .size(udec64!(10))
            .deposit(udec128!(100)) // 50x leverage
            .build();

        // Position 2: Less leveraged (20x), needs 50
        let pos2 = PositionBuilder::new()
            .perpetual_id(2)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50)) // 20x leverage
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(100)) // Only 100 available
            .position(pos1)
            .position(pos2)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        // Position 1 needs more (400 > 50), so gets all available capital
        assert_eq!(action.perpetual_id, 1);
        assert_eq!(action.amount, udec128!(100));
    }

    // ==================== PnL affecting leverage ====================

    #[test]
    fn test_position_with_loss_higher_leverage() {
        // Position with loss increases effective leverage
        // notional = 1000, deposit = 100, delta_pnl = -50
        // equity = 100 - 50 = 50, leverage = 1000/50 = 20x
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .delta_pnl(dec256!(-50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .position(position)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        // With loss, effective leverage is 20x, which triggers
        assert_eq!(action.perpetual_id, 1);
    }

    #[test]
    fn test_position_with_profit_lower_leverage() {
        // Position with profit decreases effective leverage
        // notional = 1000, deposit = 100, delta_pnl = +400
        // equity = 100 + 400 = 500, leverage = 1000/500 = 2x (under threshold)
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .delta_pnl(dec256!(400))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .position(position)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        // With profit, effective leverage is 2x, no action needed
        assert!(action.is_none());
    }

    // ==================== Underwater positions ====================

    #[test]
    fn test_underwater_position_skipped() {
        // Position with negative equity (underwater)
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .delta_pnl(dec256!(-100)) // Loss exceeds deposit
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .position(position)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&account, &config);

        // Underwater positions are skipped (leverage is None)
        assert!(action.is_none());
    }

    // ==================== evaluate_all diagnostics ====================

    #[test]
    fn test_evaluate_all_with_mixed_positions() {
        // Position 1: Under threshold (5x)
        let pos1 = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(200))
            .build();

        // Position 2: Over threshold (20x)
        let pos2 = PositionBuilder::new()
            .perpetual_id(2)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        // Position 3: Over threshold (20x)
        let pos3 = PositionBuilder::new()
            .perpetual_id(3)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(80))
            .position(pos1)
            .position(pos2)
            .position(pos3)
            .build();

        let config = make_config(udec64!(15), udec64!(10));
        let summary = evaluate_all(&account, &config);

        assert_eq!(summary.positions_evaluated, 3);
        assert_eq!(summary.over_leveraged_count, 2); // pos2 and pos3
        assert_eq!(summary.total_capital_needed, udec128!(100)); // 50 + 50
        assert_eq!(summary.available_capital, udec128!(80));
        // Both can be topped up (at least partially)
        assert_eq!(summary.positions_that_can_topup, 2);
    }

    // ==================== Reserve balance ====================

    #[test]
    fn test_reserve_balance_reduces_available() {
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50)) // 20x leverage
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(100)) // 100 total
            .position(position)
            .build();

        let config = TopUpConfig {
            trigger_leverage: udec64!(15),
            target_leverage: udec64!(10),
            perpetual_ids: vec![],
            min_reserve_balance: udec128!(80), // Reserve 80, only 20 available
        };

        let action = compute_topup(&account, &config);

        // Need 50 but only 20 available after reserve - still do partial
        assert!(action.is_some());
        let action = action.unwrap();
        assert_eq!(action.amount, udec128!(20));
    }

    // ==================== Perpetual ID filtering ====================

    #[test]
    fn test_perpetual_id_filter() {
        // Position 1: Over threshold, perp 1
        let pos1 = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        // Position 2: Over threshold, perp 2
        let pos2 = PositionBuilder::new()
            .perpetual_id(2)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(500))
            .position(pos1)
            .position(pos2)
            .build();

        let config = TopUpConfig {
            trigger_leverage: udec64!(15),
            target_leverage: udec64!(10),
            perpetual_ids: vec![1], // Only monitor perp 1
            min_reserve_balance: UD128::ZERO,
        };

        let action = compute_topup(&account, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        // Should only act on perp 1
        assert_eq!(action.perpetual_id, 1);
    }
}
