//! Pure calculation functions for margin and leverage.
//!
//! These functions are stateless and side-effect free, making them easy to test.
//! All leverage/margin calculations happen here.

use dex_sdk::state::Position;
use fastnum::{D256, UD64, UD128};

/// Calculate the equity of a position.
///
/// Equity = deposit + delta_pnl + premium_pnl
///
/// This is the actual capital value of the position in "capital space",
/// accounting for all unrealized PnL.
///
/// Note: This can be negative if the position is underwater.
pub fn equity(position: &Position) -> D256 {
    position.deposit().to_signed().resize() + position.delta_pnl() + position.premium_pnl()
}

/// Calculate the notional value of a position.
///
/// Notional = entry_price * size
///
/// This represents the total exposure of the position.
pub fn notional_value(position: &Position) -> UD128 {
    position.entry_price().resize() * position.size().resize()
}

/// Calculate the current leverage of a position.
///
/// Leverage = notional / equity
///
/// Returns None if:
/// - Equity is zero (division by zero)
/// - Equity is negative (position is underwater, leverage undefined)
///
/// A higher leverage means more risk.
pub fn current_leverage(position: &Position) -> Option<UD64> {
    let eq = equity(position);

    // Can't calculate leverage if equity is zero or negative
    if eq <= D256::ZERO {
        return None;
    }

    let notional = notional_value(position);

    // leverage = notional / equity
    let leverage_d256 = notional.to_signed().resize() / eq;

    // Convert to UD64, clamping to max if overflow
    Some(leverage_d256.unsigned_abs().resize())
}

/// Calculate the amount of collateral needed to achieve a target leverage.
///
/// Given:
///   current_equity = deposit + delta_pnl + premium_pnl
///   target_leverage = notional / target_equity
///
/// Solving for required additional deposit:
///   target_equity = notional / target_leverage
///   additional_deposit = target_equity - current_equity
///
/// Returns None if:
/// - Target leverage is zero (invalid)
/// - Current equity already achieves or exceeds target leverage
/// - Position is underwater (current equity <= 0)
pub fn required_topup_amount(position: &Position, target_leverage: UD64) -> Option<UD128> {
    if target_leverage == UD64::ZERO {
        return None;
    }

    let current_eq = equity(position);

    // If underwater, can't reasonably compute top-up
    // (would need to cover the loss first)
    if current_eq <= D256::ZERO {
        return None;
    }

    let notional = notional_value(position);

    // target_equity = notional / target_leverage
    let target_eq = notional.to_signed().resize() / target_leverage.to_signed().resize();

    // additional_deposit = target_equity - current_equity
    let additional = target_eq - current_eq;

    if additional <= D256::ZERO {
        // Already at or below target leverage, no top-up needed
        None
    } else {
        Some(additional.unsigned_abs().resize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dex_sdk::state::PositionType;
    use dex_sdk::testing::PositionBuilder;
    use fastnum::{dec256, udec64, udec128};

    // ==================== equity() tests ====================

    #[test]
    fn test_equity_deposit_only() {
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(500));
    }

    #[test]
    fn test_equity_with_positive_delta_pnl() {
        // Price moved in favor (long position, price went up)
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .delta_pnl(dec256!(200))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(700)); // 500 + 200
    }

    #[test]
    fn test_equity_with_negative_delta_pnl() {
        // Price moved against (long position, price went down)
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .delta_pnl(dec256!(-200))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(300)); // 500 - 200
    }

    #[test]
    fn test_equity_with_positive_premium_pnl() {
        // Received funding (short position during positive funding rate)
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .premium_pnl(dec256!(50))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(550)); // 500 + 50
    }

    #[test]
    fn test_equity_with_negative_premium_pnl() {
        // Paid funding (long position during positive funding rate)
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .premium_pnl(dec256!(-50))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(450)); // 500 - 50
    }

    #[test]
    fn test_equity_with_all_components() {
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .delta_pnl(dec256!(100))
            .premium_pnl(dec256!(-30))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(570)); // 500 + 100 - 30
    }

    #[test]
    fn test_equity_underwater_position() {
        // Large loss makes equity negative
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(200))
            .delta_pnl(dec256!(-300))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(-100)); // 200 - 300 = -100
    }

    // ==================== notional_value() tests ====================

    #[test]
    fn test_notional_value_basic() {
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .build();

        let notional = notional_value(&pos);
        assert_eq!(notional, udec128!(1000)); // 100 * 10
    }

    #[test]
    fn test_notional_value_large() {
        let pos = PositionBuilder::new()
            .entry_price(udec64!(50000))
            .size(udec64!(2))
            .deposit(udec128!(10000))
            .build();

        let notional = notional_value(&pos);
        assert_eq!(notional, udec128!(100000)); // 50000 * 2
    }

    // ==================== current_leverage() tests ====================

    #[test]
    fn test_leverage_basic() {
        // notional = 1000, equity = 500 → leverage = 2x
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .build();

        let lev = current_leverage(&pos).unwrap();
        assert_eq!(lev, udec64!(2));
    }

    #[test]
    fn test_leverage_high() {
        // notional = 1000, equity = 100 → leverage = 10x
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .build();

        let lev = current_leverage(&pos).unwrap();
        assert_eq!(lev, udec64!(10));
    }

    #[test]
    fn test_leverage_with_profit() {
        // notional = 1000, equity = 500 + 250 = 750 → leverage = 1000/750 ≈ 1.33
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .delta_pnl(dec256!(250))
            .build();

        let lev = current_leverage(&pos).unwrap();
        // 1000 / 750 = 1.333...
        assert!(lev > udec64!(1) && lev < udec64!(2));
    }

    #[test]
    fn test_leverage_with_loss() {
        // notional = 1000, equity = 500 - 250 = 250 → leverage = 4x
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .delta_pnl(dec256!(-250))
            .build();

        let lev = current_leverage(&pos).unwrap();
        assert_eq!(lev, udec64!(4));
    }

    #[test]
    fn test_leverage_zero_equity() {
        // Equity exactly zero
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(200))
            .delta_pnl(dec256!(-200))
            .build();

        assert!(current_leverage(&pos).is_none());
    }

    #[test]
    fn test_leverage_negative_equity() {
        // Underwater position
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(200))
            .delta_pnl(dec256!(-300))
            .build();

        assert!(current_leverage(&pos).is_none());
    }

    // ==================== required_topup_amount() tests ====================

    #[test]
    fn test_topup_basic() {
        // Current: notional=1000, equity=100, leverage=10x
        // Target: 5x leverage
        // Target equity = 1000/5 = 200
        // Top-up needed = 200 - 100 = 100
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .build();

        let topup = required_topup_amount(&pos, udec64!(5)).unwrap();
        assert_eq!(topup, udec128!(100));
    }

    #[test]
    fn test_topup_from_15x_to_10x() {
        // Current: notional=1500, equity=100, leverage=15x
        // Target: 10x leverage
        // Target equity = 1500/10 = 150
        // Top-up needed = 150 - 100 = 50
        let pos = PositionBuilder::new()
            .entry_price(udec64!(150))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .build();

        let topup = required_topup_amount(&pos, udec64!(10)).unwrap();
        assert_eq!(topup, udec128!(50));
    }

    #[test]
    fn test_topup_already_under_target() {
        // Current: notional=1000, equity=500, leverage=2x
        // Target: 5x leverage (already under target)
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(500))
            .build();

        assert!(required_topup_amount(&pos, udec64!(5)).is_none());
    }

    #[test]
    fn test_topup_exactly_at_target() {
        // Current: notional=1000, equity=200, leverage=5x
        // Target: 5x leverage (exactly at target)
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(200))
            .build();

        assert!(required_topup_amount(&pos, udec64!(5)).is_none());
    }

    #[test]
    fn test_topup_underwater_position() {
        // Can't top up an underwater position (would need to cover loss first)
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .delta_pnl(dec256!(-200))
            .build();

        assert!(required_topup_amount(&pos, udec64!(5)).is_none());
    }

    #[test]
    fn test_topup_zero_target_leverage() {
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .build();

        assert!(required_topup_amount(&pos, UD64::ZERO).is_none());
    }

    #[test]
    fn test_topup_with_pnl_components() {
        // Current: notional=1000, equity = 100 + 50 - 20 = 130, leverage ≈ 7.69x
        // Target: 5x leverage
        // Target equity = 1000/5 = 200
        // Top-up needed = 200 - 130 = 70
        let pos = PositionBuilder::new()
            .entry_price(udec64!(100))
            .size(udec64!(10))
            .deposit(udec128!(100))
            .delta_pnl(dec256!(50))
            .premium_pnl(dec256!(-20))
            .build();

        let topup = required_topup_amount(&pos, udec64!(5)).unwrap();
        assert_eq!(topup, udec128!(70));
    }

    // ==================== Real-world scenario tests ====================

    #[test]
    fn test_scenario_btc_position_normal() {
        // BTC position: 0.1 BTC at $50,000 entry
        // Deposit: $500 (10x leverage initially)
        // Price moved to $48,000 (loss)
        // delta_pnl = (48000 - 50000) * 0.1 = -200
        let pos = PositionBuilder::new()
            .entry_price(udec64!(50000))
            .size(udec64!(0.1))
            .deposit(udec128!(500))
            .delta_pnl(dec256!(-200))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(300)); // 500 - 200

        let notional = notional_value(&pos);
        assert_eq!(notional, udec128!(5000)); // 50000 * 0.1

        let lev = current_leverage(&pos).unwrap();
        // 5000 / 300 = 16.67x
        assert!(lev > udec64!(16) && lev < udec64!(17));

        // If trigger is 15x and target is 10x:
        // Target equity = 5000/10 = 500
        // Top-up = 500 - 300 = 200
        let topup = required_topup_amount(&pos, udec64!(10)).unwrap();
        assert_eq!(topup, udec128!(200));
    }

    #[test]
    fn test_scenario_eth_position_with_funding() {
        // ETH position: 1 ETH at $3,000 entry
        // Deposit: $300 (10x leverage initially)
        // Price unchanged, but paid $30 in funding
        let pos = PositionBuilder::new()
            .entry_price(udec64!(3000))
            .size(udec64!(1))
            .deposit(udec128!(300))
            .premium_pnl(dec256!(-30))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(270)); // 300 - 30

        let notional = notional_value(&pos);
        assert_eq!(notional, udec128!(3000));

        let lev = current_leverage(&pos).unwrap();
        // 3000 / 270 = 11.11x
        assert!(lev > udec64!(11) && lev < udec64!(12));
    }

    #[test]
    fn test_scenario_short_position_profit() {
        // Short ETH: 1 ETH at $3,000 entry
        // Deposit: $300 (10x leverage initially)
        // Price dropped to $2,700 (profit for short)
        // delta_pnl = (3000 - 2700) * 1 = 300
        let pos = PositionBuilder::new()
            .position_type(PositionType::Short)
            .entry_price(udec64!(3000))
            .size(udec64!(1))
            .deposit(udec128!(300))
            .delta_pnl(dec256!(300))
            .build();

        let eq = equity(&pos);
        assert_eq!(eq, dec256!(600)); // 300 + 300

        let lev = current_leverage(&pos).unwrap();
        // 3000 / 600 = 5x
        assert_eq!(lev, udec64!(5));

        // Already under 10x target, no top-up needed
        assert!(required_topup_amount(&pos, udec64!(10)).is_none());
    }
}
