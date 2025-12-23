use dex_sdk::types::PerpetualId;
use fastnum::{UD64, UD128};

/// Configuration for the top-up logic (pure data, no IO concerns).
#[derive(Clone, Debug)]
pub struct TopUpConfig {
    /// Leverage threshold that triggers a top-up.
    /// When current_leverage > trigger_leverage, position needs top-up.
    pub trigger_leverage: UD64,

    /// Target leverage after top-up.
    /// We add enough collateral to bring leverage down to this level.
    pub target_leverage: UD64,

    /// Perpetual IDs to monitor. Empty means monitor all.
    pub perpetual_ids: Vec<PerpetualId>,

    /// Minimum balance to keep in reserve (not used for top-ups).
    pub min_reserve_balance: UD128,
}

/// A single top-up action computed by the pure functional core.
#[derive(Clone, Debug, PartialEq)]
pub struct TopUpAction {
    /// The perpetual ID of the position to top up.
    pub perpetual_id: PerpetualId,

    /// Amount of collateral to add.
    pub amount: UD128,

    /// Current leverage before top-up.
    pub current_leverage: UD64,

    /// Target leverage after top-up.
    pub target_leverage: UD64,
}

/// Information about a position's margin state (for logging/diagnostics).
#[derive(Clone, Debug)]
pub struct PositionMarginInfo {
    pub perpetual_id: PerpetualId,
    pub current_leverage: Option<UD64>,
    pub is_over_leveraged: bool,
    pub required_topup: Option<UD128>,
    pub can_topup: bool,
}

/// Result of evaluating all positions (for logging/diagnostics).
/// The pure core returns Option<TopUpAction>, but this provides context.
#[derive(Clone, Debug)]
pub struct EvaluationSummary {
    pub positions_evaluated: usize,
    pub over_leveraged_count: usize,
    pub positions_that_can_topup: usize,
    pub total_capital_needed: UD128,
    pub available_capital: UD128,
    pub position_infos: Vec<PositionMarginInfo>,
}
