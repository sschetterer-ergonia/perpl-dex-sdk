//! Pure functional core for margin top-up decisions.
//!
//! This module contains the main decision-making logic for the top-up bot.
//! All functions are pure - they take state as input and return decisions,
//! with NO side effects (no IO, no logging, no state mutation).

use std::collections::HashMap;

use dex_sdk::state::{Account, Position};
use dex_sdk::types::AccountId;
use fastnum::{UD64, UD128};

use super::calc;
use super::types::{EvaluationSummary, PositionMarginInfo, TopUpAction, TopUpConfig};

/// Compute a single top-up action (or None) based on current state.
///
/// This is the main entry point for the pure functional core.
///
/// Logic:
/// 1. Collect all positions from tracked accounts
/// 2. Calculate leverage for each position
/// 3. Filter to over-leveraged positions (current_leverage > trigger_leverage)
/// 4. Sort by leverage descending (most leveraged first - greedy)
/// 5. Find first position that can be topped up with available capital
/// 6. Return Some(TopUpAction) or None
///
/// This function does NO IO, NO logging - pure computation only.
pub fn compute_topup(
    accounts: &HashMap<AccountId, Account>,
    config: &TopUpConfig,
) -> Option<TopUpAction> {
    let available_capital = calculate_available_capital(accounts, config);

    // Collect positions with their leverage, filtering to monitored perpetuals
    let mut positions_with_leverage: Vec<(&Position, UD64)> = accounts
        .values()
        .flat_map(|acc| acc.positions().values())
        .filter(|pos| {
            config.perpetual_ids.is_empty() || config.perpetual_ids.contains(&pos.perpetual_id())
        })
        .filter_map(|pos| {
            calc::current_leverage(pos).map(|lev| (pos, lev))
        })
        .filter(|(_, lev)| *lev > config.trigger_leverage)
        .collect();

    // Sort by leverage descending (most leveraged first)
    positions_with_leverage.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Find first position we can top up
    for (position, current_leverage) in positions_with_leverage {
        if let Some(required_amount) = calc::required_topup_amount(position, config.target_leverage)
        {
            if required_amount <= available_capital {
                return Some(TopUpAction {
                    perpetual_id: position.perpetual_id(),
                    amount: required_amount,
                    current_leverage,
                    target_leverage: config.target_leverage,
                });
            }
            // Not enough capital for this one, try next (less leveraged)
        }
    }

    None
}

/// Compute a full evaluation summary for logging/diagnostics.
///
/// This provides detailed information about all positions, not just the one
/// we'll act on. Useful for logging and monitoring.
pub fn evaluate_all(
    accounts: &HashMap<AccountId, Account>,
    config: &TopUpConfig,
) -> EvaluationSummary {
    let available_capital = calculate_available_capital(accounts, config);

    let mut position_infos: Vec<PositionMarginInfo> = Vec::new();
    let mut total_capital_needed = UD128::ZERO;

    for account in accounts.values() {
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

            let can_topup = required_topup
                .map(|amount| amount <= available_capital)
                .unwrap_or(false);

            position_infos.push(PositionMarginInfo {
                perpetual_id: position.perpetual_id(),
                current_leverage,
                is_over_leveraged,
                required_topup,
                can_topup,
            });
        }
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
/// This is the sum of all tracked account balances minus the reserve.
fn calculate_available_capital(
    accounts: &HashMap<AccountId, Account>,
    config: &TopUpConfig,
) -> UD128 {
    let total_balance: UD128 = accounts.values().map(|acc| acc.balance()).sum();

    // Saturating subtraction - don't go below zero
    if total_balance > config.min_reserve_balance {
        total_balance - config.min_reserve_balance
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
    fn test_calculate_available_capital_empty() {
        let accounts = HashMap::new();
        let config = make_config(udec64!(15), udec64!(10));

        let available = calculate_available_capital(&accounts, &config);
        assert_eq!(available, UD128::ZERO);
    }

    #[test]
    fn test_config_with_reserve_saturates_to_zero() {
        let accounts = HashMap::new();
        let config = TopUpConfig {
            trigger_leverage: udec64!(15),
            target_leverage: udec64!(10),
            perpetual_ids: vec![],
            min_reserve_balance: udec128!(100),
        };

        let available = calculate_available_capital(&accounts, &config);
        assert_eq!(available, UD128::ZERO);
    }

    #[test]
    fn test_compute_topup_no_accounts_returns_none() {
        let accounts = HashMap::new();
        let config = make_config(udec64!(15), udec64!(10));

        let action = compute_topup(&accounts, &config);
        assert!(action.is_none());
    }

    #[test]
    fn test_evaluate_all_no_accounts_empty_summary() {
        let accounts = HashMap::new();
        let config = make_config(udec64!(15), udec64!(10));

        let summary = evaluate_all(&accounts, &config);
        assert_eq!(summary.positions_evaluated, 0);
        assert_eq!(summary.over_leveraged_count, 0);
        assert_eq!(summary.total_capital_needed, UD128::ZERO);
        assert_eq!(summary.available_capital, UD128::ZERO);
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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

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
    fn test_single_position_over_threshold_insufficient_capital() {
        // Position at 20x leverage needs 50 top-up, but only 30 available
        let position = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(30)) // Not enough capital
            .position(position)
            .build();

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

        // No action because not enough capital
        assert!(action.is_none());
    }

    // ==================== Multiple positions - prioritization ====================

    #[test]
    fn test_multiple_positions_most_leveraged_first() {
        // Position 1: 20x leverage (needs 50)
        let pos1 = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        // Position 2: 25x leverage (needs more - should be first)
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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        // Position 2 is more leveraged (25x > 20x), should be processed first
        assert_eq!(action.perpetual_id, 2);
    }

    #[test]
    fn test_multiple_positions_skip_to_affordable() {
        // Position 1: Very leveraged but needs 200 (more than available)
        let pos1 = PositionBuilder::new()
            .perpetual_id(1)
            .entry_price(udec64!(500))
            .size(udec64!(10))
            .deposit(udec128!(100)) // 50x leverage
            .build();

        // Position 2: Less leveraged but affordable
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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        // Position 1 needs too much, so we skip to position 2
        assert_eq!(action.perpetual_id, 2);
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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let action = compute_topup(&accounts, &config);

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

        // Position 2: Over threshold (20x), affordable
        let pos2 = PositionBuilder::new()
            .perpetual_id(2)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        // Position 3: Over threshold (20x), not affordable (needs 50, only 30 left after pos2)
        let pos3 = PositionBuilder::new()
            .perpetual_id(3)
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(50))
            .build();

        let account = AccountBuilder::new()
            .id(1)
            .balance(udec128!(80)) // Only enough for one top-up
            .position(pos1)
            .position(pos2)
            .position(pos3)
            .build();

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = make_config(udec64!(15), udec64!(10));
        let summary = evaluate_all(&accounts, &config);

        assert_eq!(summary.positions_evaluated, 3);
        assert_eq!(summary.over_leveraged_count, 2); // pos2 and pos3
        assert_eq!(summary.total_capital_needed, udec128!(100)); // 50 + 50
        assert_eq!(summary.available_capital, udec128!(80));
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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = TopUpConfig {
            trigger_leverage: udec64!(15),
            target_leverage: udec64!(10),
            perpetual_ids: vec![],
            min_reserve_balance: udec128!(80), // Reserve 80, only 20 available
        };

        let action = compute_topup(&accounts, &config);

        // Need 50 but only 20 available after reserve
        assert!(action.is_none());
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

        let mut accounts = HashMap::new();
        accounts.insert(1, account);

        let config = TopUpConfig {
            trigger_leverage: udec64!(15),
            target_leverage: udec64!(10),
            perpetual_ids: vec![1], // Only monitor perp 1
            min_reserve_balance: UD128::ZERO,
        };

        let action = compute_topup(&accounts, &config);

        assert!(action.is_some());
        let action = action.unwrap();
        // Should only act on perp 1
        assert_eq!(action.perpetual_id, 1);
    }
}
