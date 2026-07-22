//! Futures Roll Manager — transparent contract roll management for live trading.
//!
//! Sits between the Connector (data source) and strategy dispatch, intercepting
//! raw per-contract bars (e.g., ESH5, ESM5), maintaining per-product roll state
//! machines, and emitting synthetic bars to strategies using generic symbol
//! notation (ES=F, ES=1, ES=2).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::broker::Side;
use super::connector::LiveBar;
use super::market_calendar::MarketCalendar;
use super::product_registry::ProductRegistry;

// ─── Symbol Types ────────────────────────────────────────────────────────────

/// A generic symbol as declared by strategies (e.g., "ES=F", "NQ=2").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericSymbol {
    /// Product root (e.g., "ES", "NQ", "RTY", "YM").
    pub root: String,
    /// Symbol mode.
    pub mode: SymbolMode,
}

/// The abstraction mode for a generic symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolMode {
    /// Backward-ratio-adjusted continuous series (`=F`).
    Continuous,
    /// Nth contract in the quarterly cycle, 1-indexed (`=1`, `=2`, `=3`, ...).
    NthMonth(u8),
}

/// A concrete exchange-traded contract (e.g., "ESH5").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConcreteContract {
    /// Product root (e.g., "ES").
    pub root: String,
    /// Month code (H, M, U, Z).
    pub month: MonthCode,
    /// Single-digit year (0–9, e.g., 5 = 2025).
    pub year: u8,
}

/// CME quarterly month codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MonthCode {
    /// March
    H = 0,
    /// June
    M = 1,
    /// September
    U = 2,
    /// December
    Z = 3,
}

impl fmt::Display for MonthCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ch = match self {
            MonthCode::H => "H",
            MonthCode::M => "M",
            MonthCode::U => "U",
            MonthCode::Z => "Z",
        };
        write!(f, "{}", ch)
    }
}

/// Result of parsing either symbol type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// A strategy-facing generic symbol (e.g., "ES=F", "NQ=2").
    Generic(GenericSymbol),
    /// An exchange-traded concrete contract (e.g., "ESH5").
    Concrete(ConcreteContract),
}

// ─── Roll State Types ────────────────────────────────────────────────────────

/// Roll state machine phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollPhase {
    /// Normal operation — L1 is the active front month.
    Active,
    /// Roll triggered — transitioning contracts.
    Rolling,
}

/// Signal emitted when the 5-day average volume crossover is detected.
///
/// Contains all the information needed to decide whether and how to execute
/// a position roll, including old/new contracts and volume evidence.
#[derive(Debug, Clone)]
pub struct RollSignal {
    /// Product root for which the crossover was detected (e.g., "ES").
    pub product_root: String,
    /// The outgoing front-month contract being rolled away from.
    pub old_l1: ConcreteContract,
    /// The new front-month contract (previously L2) being rolled into.
    pub new_l1: ConcreteContract,
    /// The new L2 contract (next in the quarterly cycle after new L1).
    pub new_l2: ConcreteContract,
    /// 5-day average volume of the outgoing L1 at crossover time.
    pub l1_avg_volume: f64,
    /// 5-day average volume of the incoming L1 (old L2) at crossover time.
    pub l2_avg_volume: f64,
    /// Date of the session close that triggered the crossover.
    pub trigger_date: NaiveDate,
}

/// Event sent to the harness to execute a position roll.
///
/// The harness fills in position details (qty, direction) from the position
/// tracker and submits the roll via the broker adapter.
#[derive(Debug, Clone)]
pub struct RollEvent {
    /// Product root being rolled (e.g., "ES").
    pub product_root: String,
    /// Contract being closed out.
    pub old_contract: ConcreteContract,
    /// Contract being opened.
    pub new_contract: ConcreteContract,
    /// Quantity to roll (filled by harness from position tracker).
    pub position_qty: f64,
    /// Direction of the existing position (filled by harness).
    pub direction: Side,
    /// Price ratio between new and old contract at roll time.
    pub adjustment_ratio: f64,
    /// Whether to use a calendar spread order (atomic roll) vs. two legs.
    pub use_calendar_spread: bool,
}

/// Record of a completed roll (for history/persistence).
///
/// Stored in roll history and serialized with checkpoint state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollRecord {
    /// Date the roll was executed.
    pub date: NaiveDate,
    /// Product root that rolled (e.g., "ES").
    pub product_root: String,
    /// The old front-month contract string (e.g., "ESH5").
    pub old_contract: String,
    /// The new front-month contract string (e.g., "ESM5").
    pub new_contract: String,
    /// Price ratio (new_close / old_close) applied at roll time.
    pub adjustment_ratio: f64,
    /// Whether a position was actually rolled (false if flat at roll time).
    pub position_rolled: bool,
}

/// Individual adjustment at a roll point.
///
/// Records a single ratio adjustment entry in the continuous adjuster history,
/// used for reconstruction and audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjustmentRecord {
    /// Date the adjustment was applied.
    pub date: NaiveDate,
    /// Contract being rolled out of (e.g., "ESH5").
    pub old_contract: String,
    /// Contract being rolled into (e.g., "ESM5").
    pub new_contract: String,
    /// The price ratio for this roll (new_close / old_close).
    pub ratio: f64,
    /// Cumulative adjustment factor after applying this ratio.
    pub cumulative_factor_after: f64,
}

/// Current contract mapping for a product (query/monitoring).
///
/// Snapshot of the roll manager's state for a single product, used by
/// the harness for logging and by the broker adapter for order routing.
#[derive(Debug, Clone)]
pub struct ContractMapping {
    /// Product root (e.g., "ES").
    pub product_root: String,
    /// Current front-month (active) contract.
    pub l1: ConcreteContract,
    /// Next contract in the quarterly cycle (monitored for volume crossover).
    pub l2: ConcreteContract,
    /// Product of all historical adjustment ratios applied to this series.
    pub cumulative_adjustment_factor: f64,
    /// Whether the state machine is in Active or Rolling phase.
    pub phase: RollPhase,
}

/// Subscription record linking a strategy to a generic symbol.
///
/// Registered during strategy initialization via `register_subscription`.
/// The roll manager uses these to determine which synthetic bars to emit.
#[derive(Debug, Clone)]
pub struct SymbolSubscription {
    /// Name of the subscribing strategy (for logging and diagnostics).
    pub strategy_name: String,
    /// The generic symbol this strategy wants to receive bars for.
    pub generic_symbol: GenericSymbol,
}

// ─── Error Type ──────────────────────────────────────────────────────────────

/// Errors from the futures roll module.
#[derive(Debug, thiserror::Error)]
pub enum RollError {
    /// The symbol string could not be parsed as either a generic or concrete contract.
    #[error("invalid symbol: {0}")]
    InvalidSymbol(String),

    /// The product root is not registered in the roll manager.
    #[error("unknown product root: {0}")]
    UnknownProduct(String),

    /// State restoration from a checkpoint failed (version mismatch or corrupt data).
    #[error("state restoration failed: {0}")]
    StateRestore(String),

    /// A roll execution failed (e.g., zero close price makes ratio undefined).
    #[error("roll execution failed for {product}: {reason}")]
    RollFailed { product: String, reason: String },

    /// A calendar spread order to the broker was rejected or timed out.
    #[error("calendar spread order failed: {0}")]
    SpreadOrderFailed(String),
}

// ─── Process Result ──────────────────────────────────────────────────────────

/// Result of processing a single bar through the roll manager.
#[derive(Debug, Clone)]
pub struct ProcessResult {
    /// Synthetic bars to dispatch to strategies.
    pub bars: Vec<LiveBar>,
    /// Roll event if a roll was triggered (None most of the time).
    pub roll_event: Option<RollEvent>,
}

// ─── Persistence Types ───────────────────────────────────────────────────────

/// Serializable state for persistence across restarts.
///
/// Captured by `snapshot_state()` and restored by `restore_state()` to resume
/// roll management without re-triggering historical rolls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuturesRollState {
    /// Schema version for forward compatibility (currently 1).
    pub version: u32,
    /// Per-product: current L1, L2, volume buffers, roll_latched.
    pub machines: Vec<SerializedRollMachine>,
    /// Per-product: cumulative factor + adjustment history.
    pub adjusters: Vec<SerializedAdjuster>,
    /// Complete roll history.
    pub roll_history: Vec<RollRecord>,
}

/// Serialized per-product roll state machine.
///
/// Contains the minimal state needed to reconstruct a `RollStateMachine`
/// without replaying historical data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedRollMachine {
    /// Product root (e.g., "ES").
    pub product_root: String,
    /// Current L1 contract as a string (e.g., "ESM5").
    pub l1: String,
    /// Current L2 contract as a string (e.g., "ESU5").
    pub l2: String,
    /// Saved L1 volume buffer contents (oldest first).
    pub l1_volumes: Vec<u64>,
    /// Saved L2 volume buffer contents (oldest first).
    pub l2_volumes: Vec<u64>,
    /// Whether the roll latch was engaged.
    pub roll_latched: bool,
    /// State machine phase ("active" or "rolling").
    pub phase: String,
}

/// Serialized per-product continuous adjuster.
///
/// Stores the cumulative factor and full adjustment history for reconstruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedAdjuster {
    /// Product root (e.g., "ES").
    pub product_root: String,
    /// Cumulative adjustment factor at time of serialization.
    pub cumulative_factor: f64,
    /// Ordered history of individual roll adjustments.
    pub adjustments: Vec<AdjustmentRecord>,
}

// ─── Volume Buffer ───────────────────────────────────────────────────────────

/// Rolling 5-day volume buffer (circular).
#[derive(Debug, Clone, Default)]
pub struct VolumeBuffer {
    /// Circular buffer of daily volumes.
    days: [u64; 5],
    /// Number of days populated (0..=5).
    count: u8,
    /// Write index.
    index: usize,
}

impl VolumeBuffer {
    /// Create a new empty volume buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new daily volume into the buffer.
    pub fn push(&mut self, daily_volume: u64) {
        self.days[self.index] = daily_volume;
        self.index = (self.index + 1) % 5;
        if self.count < 5 {
            self.count += 1;
        }
    }

    /// Compute the average of stored volumes, or None if buffer is not full (< 5 days).
    /// Returns the arithmetic mean of all 5 values when the buffer is full.
    pub fn average(&self) -> Option<f64> {
        if self.count < 5 {
            return None;
        }
        let sum: u64 = self.days.iter().sum();
        Some(sum as f64 / 5.0)
    }

    /// Whether the buffer has a full 5 days of history.
    pub fn is_full(&self) -> bool {
        self.count >= 5
    }

    /// Get the stored volumes as a Vec (for serialization).
    ///
    /// Returns volumes in insertion order (oldest first).
    pub fn to_vec(&self) -> Vec<u64> {
        if self.count < 5 {
            self.days[..self.count as usize].to_vec()
        } else {
            // Circular buffer: oldest is at self.index (the next write position)
            let mut result = Vec::with_capacity(5);
            for i in 0..5 {
                result.push(self.days[(self.index + i) % 5]);
            }
            result
        }
    }

    /// Restore from a serialized Vec.
    ///
    /// Replays each value through `push()` to reconstruct the circular buffer state.
    pub fn from_vec(volumes: &[u64]) -> Self {
        let mut buf = Self::new();
        for &v in volumes {
            buf.push(v);
        }
        buf
    }
}

// ─── Symbol Parsing ──────────────────────────────────────────────────────────

/// Type alias for parse errors — uses `RollError::InvalidSymbol`.
pub type ParseError = RollError;

/// Parse a symbol string into either a GenericSymbol or ConcreteContract.
///
/// Dispatches to `parse_generic` if the input contains `=`, otherwise tries `parse_concrete`.
pub fn parse_symbol(input: &str) -> Result<SymbolKind, ParseError> {
    if input.contains('=') {
        parse_generic(input).map(SymbolKind::Generic)
    } else {
        parse_concrete(input).map(SymbolKind::Concrete)
    }
}

/// Parse a generic symbol string (e.g., "ES=F", "NQ=2").
///
/// Pattern: 1–4 uppercase ASCII letters + `=` + (`F` | positive integer).
pub fn parse_generic(input: &str) -> Result<GenericSymbol, ParseError> {
    let parts: Vec<&str> = input.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(RollError::InvalidSymbol(format!(
            "generic symbol must contain '=': '{input}'"
        )));
    }

    let root = parts[0];
    let mode_str = parts[1];

    // Validate root: 1–4 uppercase ASCII letters
    if root.is_empty() || root.len() > 4 {
        return Err(RollError::InvalidSymbol(format!(
            "product root must be 1–4 uppercase letters, got '{root}'"
        )));
    }
    if !root.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(RollError::InvalidSymbol(format!(
            "product root must be all uppercase ASCII letters, got '{root}'"
        )));
    }

    // Validate mode: "F" or a positive integer
    let mode = if mode_str == "F" {
        SymbolMode::Continuous
    } else {
        match mode_str.parse::<u8>() {
            Ok(n) if n > 0 => SymbolMode::NthMonth(n),
            _ => {
                return Err(RollError::InvalidSymbol(format!(
                    "mode must be 'F' or a positive integer, got '{mode_str}'"
                )));
            }
        }
    };

    Ok(GenericSymbol {
        root: root.to_string(),
        mode,
    })
}

/// Parse a concrete contract symbol (e.g., "ESH5", "RTYU5").
///
/// Pattern: 1–4 uppercase ASCII letters + month code (H/M/U/Z) + year digit (0–9).
pub fn parse_concrete(input: &str) -> Result<ConcreteContract, ParseError> {
    let len = input.len();

    // Minimum: 1 root char + 1 month char + 1 year digit = 3
    if len < 3 {
        return Err(RollError::InvalidSymbol(format!(
            "concrete contract too short (min 3 chars): '{input}'"
        )));
    }

    // Last char is the year digit
    let year_char = input.as_bytes()[len - 1] as char;
    if !year_char.is_ascii_digit() {
        return Err(RollError::InvalidSymbol(format!(
            "last character must be a digit (year), got '{year_char}' in '{input}'"
        )));
    }
    let year = year_char as u8 - b'0';

    // Second to last char is the month code
    let month_char = input.as_bytes()[len - 2] as char;
    let month = match month_char {
        'H' => MonthCode::H,
        'M' => MonthCode::M,
        'U' => MonthCode::U,
        'Z' => MonthCode::Z,
        _ => {
            return Err(RollError::InvalidSymbol(format!(
                "month code must be H, M, U, or Z, got '{month_char}' in '{input}'"
            )));
        }
    };

    // Everything before the last 2 chars is the root
    let root = &input[..len - 2];

    // Validate root: 1–4 uppercase ASCII letters
    if root.is_empty() || root.len() > 4 {
        return Err(RollError::InvalidSymbol(format!(
            "product root must be 1–4 uppercase letters, got '{root}' in '{input}'"
        )));
    }
    if !root.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(RollError::InvalidSymbol(format!(
            "product root must be all uppercase ASCII letters, got '{root}' in '{input}'"
        )));
    }

    Ok(ConcreteContract {
        root: root.to_string(),
        month,
        year,
    })
}

/// Format a GenericSymbol back to its canonical string representation.
///
/// Produces "{root}={mode}" where mode is "F" for Continuous or the integer for NthMonth.
pub fn format_generic(symbol: &GenericSymbol) -> String {
    match symbol.mode {
        SymbolMode::Continuous => format!("{}=F", symbol.root),
        SymbolMode::NthMonth(n) => format!("{}={}", symbol.root, n),
    }
}

/// Format a ConcreteContract back to its canonical string representation.
///
/// Produces "{root}{month}{year}" e.g. "ESH5".
pub fn format_concrete(contract: &ConcreteContract) -> String {
    format!("{}{}{}", contract.root, contract.month, contract.year)
}

// ─── Roll State Machine ─────────────────────────────────────────────────────

/// Per-product state machine tracking L1/L2 contracts and volume crossover.
///
/// Monitors 5-day average volume for the front-month (L1) and next-month (L2)
/// contracts. When L2's average exceeds L1's, a roll signal fires: L2 is
/// promoted to L1, a new L2 is computed from the quarterly cycle, and volume
/// buffers are reset for the next cycle.
pub struct RollStateMachine {
    pub(crate) product_root: String,
    pub(crate) state: RollPhase,
    pub(crate) l1: ConcreteContract,
    pub(crate) l2: ConcreteContract,
    pub(crate) l1_volume_history: VolumeBuffer,
    pub(crate) l2_volume_history: VolumeBuffer,
    /// Intraday volume accumulator (reset at session boundary).
    pub(crate) l1_intraday_volume: u64,
    pub(crate) l2_intraday_volume: u64,
    /// Whether the roll has already fired this cycle (latch).
    pub(crate) roll_latched: bool,
    /// Last known close price for L1 (used for ratio adjustment at roll time).
    pub(crate) l1_last_close: f64,
    /// Last known close price for L2 (used for ratio adjustment at roll time).
    pub(crate) l2_last_close: f64,
}

/// Result of executing a roll transition.
///
/// Describes the contract changes that occurred when `execute_roll()` was called.
#[derive(Debug, Clone)]
pub struct RollTransition {
    /// The former front-month contract that was rolled out of.
    pub old_l1: ConcreteContract,
    /// The new front-month contract (promoted from L2).
    pub new_l1: ConcreteContract,
    /// The new second-month contract (computed from the quarterly cycle).
    pub new_l2: ConcreteContract,
}

impl RollStateMachine {
    /// Create a new RollStateMachine for a given product root with specified L1 and L2 contracts.
    pub fn new(product_root: String, l1: ConcreteContract, l2: ConcreteContract) -> Self {
        Self {
            product_root,
            state: RollPhase::Active,
            l1,
            l2,
            l1_volume_history: VolumeBuffer::new(),
            l2_volume_history: VolumeBuffer::new(),
            l1_intraday_volume: 0,
            l2_intraday_volume: 0,
            roll_latched: false,
            l1_last_close: 0.0,
            l2_last_close: 0.0,
        }
    }

    /// Feed a bar's volume into the appropriate contract accumulator.
    ///
    /// If the contract matches L1, adds to l1_intraday_volume.
    /// If the contract matches L2, adds to l2_intraday_volume.
    /// Otherwise, the volume is ignored.
    pub fn accumulate_volume(&mut self, contract: &ConcreteContract, volume: u64) {
        if contract == &self.l1 {
            self.l1_intraday_volume += volume;
        } else if contract == &self.l2 {
            self.l2_intraday_volume += volume;
        }
        // Bars for other contracts are silently ignored.
    }

    /// Push intraday volume totals into the rolling buffers and reset accumulators.
    ///
    /// Called once per trading session at the session boundary (end of day).
    pub fn end_of_day(&mut self) {
        self.l1_volume_history.push(self.l1_intraday_volume);
        self.l2_volume_history.push(self.l2_intraday_volume);
        self.l1_intraday_volume = 0;
        self.l2_intraday_volume = 0;
    }

    /// Evaluate the end-of-day crossover condition.
    ///
    /// Returns `Some(RollSignal)` if all guards pass:
    /// 1. `roll_latched` is false (no roll has fired this cycle)
    /// 2. Both volume buffers are full (>= 5 days of history)
    /// 3. The 5-day average volume of L2 exceeds that of L1
    ///
    /// The `trigger_date` parameter is the date of the session close being evaluated.
    pub fn evaluate_crossover(&self, trigger_date: NaiveDate) -> Option<RollSignal> {
        // Guard: already rolled this cycle
        if self.roll_latched {
            return None;
        }

        // Guard: both buffers must be full (5 days of history)
        if !self.l1_volume_history.is_full() || !self.l2_volume_history.is_full() {
            return None;
        }

        // Get averages (safe to unwrap since we checked is_full)
        let l1_avg = self.l1_volume_history.average().unwrap();
        let l2_avg = self.l2_volume_history.average().unwrap();

        // Crossover: L2 average exceeds L1 average
        if l2_avg > l1_avg {
            let new_l2 = QuarterlyCycle::next(&self.l2);
            Some(RollSignal {
                product_root: self.product_root.clone(),
                old_l1: self.l1.clone(),
                new_l1: self.l2.clone(),
                new_l2,
                l1_avg_volume: l1_avg,
                l2_avg_volume: l2_avg,
                trigger_date,
            })
        } else {
            None
        }
    }

    /// Perform the roll: promote L2 → L1, compute new L2, set roll_latched = true.
    ///
    /// Transitions through the Rolling state and resets volume tracking for the new cycle.
    /// Returns a `RollTransition` describing the contract changes.
    pub fn execute_roll(&mut self) -> RollTransition {
        // Transition to Rolling state
        self.state = RollPhase::Rolling;

        let old_l1 = self.l1.clone();

        // Promote L2 → L1
        self.l1 = self.l2.clone();

        // Compute new L2 as next in the quarterly cycle
        self.l2 = QuarterlyCycle::next(&self.l1);

        // Latch the roll to prevent re-triggering within this same session.
        // Note: the buffer reset below provides the real guard (needs 5 fresh days
        // before the next crossover can evaluate), so we immediately unlatch after
        // the transition completes to allow the next quarterly cycle's roll.
        self.roll_latched = false;

        // Reset volume histories and intraday accumulators for the new cycle
        self.l1_volume_history = VolumeBuffer::new();
        self.l2_volume_history = VolumeBuffer::new();
        self.l1_intraday_volume = 0;
        self.l2_intraday_volume = 0;

        // Return to Active state
        self.state = RollPhase::Active;

        RollTransition {
            old_l1,
            new_l1: self.l1.clone(),
            new_l2: self.l2.clone(),
        }
    }

    /// Query the current state of the roll state machine.
    ///
    /// Returns the current phase, L1 contract, and L2 contract.
    pub fn current_state(&self) -> (RollPhase, &ConcreteContract, &ConcreteContract) {
        (self.state, &self.l1, &self.l2)
    }

    /// Get the current intraday volume for L1 (accumulated since last end_of_day).
    pub fn l1_intraday_volume(&self) -> u64 {
        self.l1_intraday_volume
    }

    /// Get the current intraday volume for L2 (accumulated since last end_of_day).
    pub fn l2_intraday_volume(&self) -> u64 {
        self.l2_intraday_volume
    }
}

// ─── MonthCode helpers ───────────────────────────────────────────────────────

impl MonthCode {
    /// The ordered quarterly cycle.
    const CYCLE: [MonthCode; 4] = [MonthCode::H, MonthCode::M, MonthCode::U, MonthCode::Z];

    /// Calendar month number for this code (1-indexed: H=3, M=6, U=9, Z=12).
    pub fn calendar_month(self) -> u32 {
        match self {
            MonthCode::H => 3,
            MonthCode::M => 6,
            MonthCode::U => 9,
            MonthCode::Z => 12,
        }
    }

    /// Index in the quarterly cycle (H=0, M=1, U=2, Z=3).
    pub fn cycle_index(self) -> usize {
        self as usize
    }

    /// Get the next month code in the cycle. Returns (next_code, year_wrapped).
    pub fn next(self) -> (MonthCode, bool) {
        match self {
            MonthCode::H => (MonthCode::M, false),
            MonthCode::M => (MonthCode::U, false),
            MonthCode::U => (MonthCode::Z, false),
            MonthCode::Z => (MonthCode::H, true),
        }
    }

    /// Get the previous month code in the cycle. Returns (prev_code, year_wrapped).
    pub fn previous(self) -> (MonthCode, bool) {
        match self {
            MonthCode::H => (MonthCode::Z, true),
            MonthCode::M => (MonthCode::H, false),
            MonthCode::U => (MonthCode::M, false),
            MonthCode::Z => (MonthCode::U, false),
        }
    }

    /// Get the MonthCode at a given cycle index (0=H, 1=M, 2=U, 3=Z).
    pub fn from_index(index: usize) -> MonthCode {
        Self::CYCLE[index % 4]
    }
}

// ─── QuarterlyCycle ──────────────────────────────────────────────────────────

/// Navigate the CME quarterly cycle (H, M, U, Z).
///
/// Provides stateless utility methods for computing the next/previous contract
/// in the cycle and finding the nearest non-expired contracts for a product root.
pub struct QuarterlyCycle;

impl QuarterlyCycle {
    /// Get the next contract in the cycle.
    /// H→M, M→U, U→Z, Z→H (year+1 with digit wrapping: 9→0).
    pub fn next(contract: &ConcreteContract) -> ConcreteContract {
        let (next_month, year_wrapped) = contract.month.next();
        let next_year = if year_wrapped {
            (contract.year + 1) % 10
        } else {
            contract.year
        };
        ConcreteContract {
            root: contract.root.clone(),
            month: next_month,
            year: next_year,
        }
    }

    /// Get the previous contract in the cycle.
    /// M→H, U→M, Z→U, H→Z (year-1 with digit wrapping: 0→9).
    pub fn previous(contract: &ConcreteContract) -> ConcreteContract {
        let (prev_month, year_wrapped) = contract.month.previous();
        let prev_year = if year_wrapped {
            if contract.year == 0 { 9 } else { contract.year - 1 }
        } else {
            contract.year
        };
        ConcreteContract {
            root: contract.root.clone(),
            month: prev_month,
            year: prev_year,
        }
    }

    /// Get the N nearest non-expired contracts for a product root.
    ///
    /// A contract is considered expired if its expiration month has ended
    /// relative to `today`. For example, H (March) is expired if today is
    /// April or later in the same year.
    pub fn nearest_contracts(root: &str, today: NaiveDate, count: usize) -> Vec<ConcreteContract> {
        if count == 0 {
            return Vec::new();
        }

        let current_month = today.month();
        let year_digit = (today.year() % 10) as u8;

        // Find the first non-expired quarterly month in the current year.
        // A contract expires at the end of its month, so it's still active if
        // we are currently in that month or earlier.
        let first_contract = Self::first_non_expired(root, current_month, year_digit);

        // Collect `count` contracts starting from the first non-expired one.
        let mut result = Vec::with_capacity(count);
        let mut current = first_contract;
        for _ in 0..count {
            result.push(current.clone());
            current = Self::next(&current);
        }

        result
    }

    /// Find the first non-expired contract given the current month and year digit.
    fn first_non_expired(root: &str, current_month: u32, year_digit: u8) -> ConcreteContract {
        // Walk through the cycle starting from H to find the first contract
        // whose expiration month is >= current_month in the current year.
        for &month_code in &MonthCode::CYCLE {
            if month_code.calendar_month() >= current_month {
                return ConcreteContract {
                    root: root.to_string(),
                    month: month_code,
                    year: year_digit,
                };
            }
        }
        // All quarterly months in the current year are expired.
        // The first non-expired is H of next year.
        ConcreteContract {
            root: root.to_string(),
            month: MonthCode::H,
            year: (year_digit + 1) % 10,
        }
    }
}

// ─── Continuous Adjuster ──────────────────────────────────────────────────────

/// Manages backward ratio adjustment for continuous futures series (`=F` symbols).
///
/// Maintains a cumulative factor that is applied to raw prices to produce a
/// gap-free continuous price series across contract rolls. Supports two modes:
///
/// - **Forward mode** (`backward_mode = false`): Used during live trading. Each roll
///   multiplies the new ratio (new_close / old_close) into the cumulative factor.
///   The factor starts at 1.0 and grows with each roll, adjusting historical prices
///   relative to the current contract.
///
/// - **Backward mode** (`backward_mode = true`): Used during replay of historical data
///   with a pre-seeded cumulative factor. The factor starts at the total product of
///   all historical ratios and is progressively divided at each roll, converging to 1.0
///   by the end of history. This ensures that the most recent prices are unadjusted
///   while older prices are scaled down.
pub struct ContinuousAdjuster {
    /// Product of all historical adjustment ratios.
    pub(crate) cumulative_factor: f64,
    /// History of individual adjustments (for persistence/reconstruction).
    pub(crate) adjustments: Vec<AdjustmentRecord>,
    /// When true, rolls DIVIDE the factor (backward mode for replay with pre-seeded factor).
    /// When false, rolls MULTIPLY the factor (forward mode for live).
    pub(crate) backward_mode: bool,
}

impl Default for ContinuousAdjuster {
    fn default() -> Self {
        Self {
            cumulative_factor: 1.0,
            adjustments: Vec::new(),
            backward_mode: false,
        }
    }
}

impl ContinuousAdjuster {
    /// Create a new adjuster with factor = 1.0 (no adjustment).
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a new roll adjustment. Returns the ratio on success, or an error if `old_close` is zero.
    ///
    /// Computes ratio = new_close / old_close, multiplies it into the cumulative factor,
    /// and records the adjustment for persistence/reconstruction.
    pub fn apply_roll(
        &mut self,
        old_close: f64,
        new_close: f64,
        date: NaiveDate,
        old_contract: &str,
        new_contract: &str,
    ) -> Result<f64, RollError> {
        if old_close == 0.0 {
            return Err(RollError::RollFailed {
                product: "unknown".to_string(),
                reason: "old_close is zero, cannot compute ratio".to_string(),
            });
        }
        let ratio = new_close / old_close;
        if self.backward_mode {
            // In backward mode, the factor was pre-seeded with the total product.
            // Each roll DIVIDES by the ratio, progressively reducing the adjustment
            // until factor reaches 1.0 at the end of history.
            self.cumulative_factor /= ratio;
        } else {
            // In forward mode (live), each roll multiplies the ratio in.
            self.cumulative_factor *= ratio;
        }
        self.adjustments.push(AdjustmentRecord {
            date,
            old_contract: old_contract.to_string(),
            new_contract: new_contract.to_string(),
            ratio,
            cumulative_factor_after: self.cumulative_factor,
        });
        Ok(ratio)
    }

    /// Adjust a raw price to the continuous series.
    ///
    /// Returns raw_price × cumulative_factor.
    pub fn adjust_price(&self, raw_price: f64) -> f64 {
        raw_price * self.cumulative_factor
    }

    /// Reverse adjustment (for order submission).
    ///
    /// Returns adjusted_price / cumulative_factor.
    pub fn unadjust_price(&self, adjusted_price: f64) -> f64 {
        adjusted_price / self.cumulative_factor
    }

    /// Adjust volume (divide by factor to preserve volume-weighted calculations).
    ///
    /// Returns raw_volume as f64 / cumulative_factor.
    pub fn adjust_volume(&self, raw_volume: u64) -> f64 {
        raw_volume as f64 / self.cumulative_factor
    }
}

// ─── Futures Roll Manager ────────────────────────────────────────────────────

/// Top-level orchestrator for futures contract roll management.
///
/// Sits between the data connector and strategy dispatch, intercepting raw
/// per-contract bars (e.g., ESH5, ESM5) and emitting synthetic bars using
/// generic symbol notation (ES=F, ES=1, ES=2). Coordinates:
///
/// - Per-product `RollStateMachine` instances that track volume crossover
/// - Per-product `ContinuousAdjuster` instances for `=F` ratio adjustment
/// - Strategy subscriptions that determine which synthetic bars to emit
/// - Roll history for observability and state persistence
pub struct FuturesRollManager {
    /// Per-product roll state machines.
    pub(crate) state_machines: HashMap<String, RollStateMachine>,
    /// Per-product continuous adjusters (for =F mode).
    pub(crate) adjusters: HashMap<String, ContinuousAdjuster>,
    /// Which generic symbols each strategy has subscribed to.
    pub(crate) subscriptions: Vec<SymbolSubscription>,
    /// Reference to product registry for tick_size, multiplier lookups.
    pub(crate) product_registry: Arc<ProductRegistry>,
    /// Reference to market calendar for session boundary detection.
    pub(crate) calendar: Arc<MarketCalendar>,
    /// Roll history for persistence and observability.
    pub(crate) roll_history: Vec<RollRecord>,
}

impl FuturesRollManager {
    /// Create a new FuturesRollManager with no active subscriptions.
    pub fn new(product_registry: Arc<ProductRegistry>, calendar: Arc<MarketCalendar>) -> Self {
        Self {
            state_machines: HashMap::new(),
            adjusters: HashMap::new(),
            subscriptions: Vec::new(),
            product_registry,
            calendar,
            roll_history: Vec::new(),
        }
    }

    /// Process an incoming raw bar. Returns synthetic bars for dispatch
    /// and optionally a RollEvent if a roll was triggered.
    ///
    /// Flow:
    /// 1. Parse the bar's symbol as a concrete contract
    /// 2. If parsing fails (not a futures symbol), pass through unmodified
    /// 3. If the root doesn't match a registered state machine, pass through
    /// 4. Otherwise, accumulate volume and emit synthetic bars per subscription
    pub fn process_bar(&mut self, bar: &LiveBar) -> ProcessResult {
        // Try to parse as a concrete contract
        let contract = match parse_concrete(&bar.bar.symbol) {
            Ok(c) => c,
            Err(_) => {
                // Not a futures symbol — pass through unmodified
                return ProcessResult {
                    bars: vec![bar.clone()],
                    roll_event: None,
                };
            }
        };

        // Check if the product root is tracked by a state machine
        let product_root = &contract.root;
        if !self.state_machines.contains_key(product_root) {
            // Not a registered product root — pass through unmodified
            return ProcessResult {
                bars: vec![bar.clone()],
                roll_event: None,
            };
        }

        // Accumulate volume on the state machine
        let sm = self.state_machines.get_mut(product_root).unwrap();
        sm.accumulate_volume(&contract, bar.bar.volume as u64);

        // Snapshot current L1/L2 for subscription matching
        let l1 = sm.l1.clone();
        let l2 = sm.l2.clone();
        let root = product_root.clone();

        // Build synthetic bars based on subscriptions
        let mut synthetic_bars = Vec::new();
        for sub in &self.subscriptions {
            if sub.generic_symbol.root != root {
                continue;
            }
            match sub.generic_symbol.mode {
                SymbolMode::Continuous => {
                    // Emit adjusted bar if this is the L1 contract
                    if contract == l1 {
                        if let Some(adjuster) = self.adjusters.get(&root) {
                            let adjusted_bar = LiveBar {
                                bar: flux_runtime::BarContext {
                                    symbol: format!("{}=F", root),
                                    open: adjuster.adjust_price(bar.bar.open),
                                    high: adjuster.adjust_price(bar.bar.high),
                                    low: adjuster.adjust_price(bar.bar.low),
                                    close: adjuster.adjust_price(bar.bar.close),
                                    volume: adjuster.adjust_volume(bar.bar.volume as u64),
                                    in_position: bar.bar.in_position,
                                },
                                connector_id: bar.connector_id.clone(),
                                received_at: bar.received_at,
                            };
                            synthetic_bars.push(adjusted_bar);
                        }
                    }
                }
                SymbolMode::NthMonth(1) => {
                    // Emit unadjusted bar if this is the L1 contract
                    if contract == l1 {
                        let nth_bar = LiveBar {
                            bar: flux_runtime::BarContext {
                                symbol: format!("{}=1", root),
                                open: bar.bar.open,
                                high: bar.bar.high,
                                low: bar.bar.low,
                                close: bar.bar.close,
                                volume: bar.bar.volume,
                                in_position: bar.bar.in_position,
                            },
                            connector_id: bar.connector_id.clone(),
                            received_at: bar.received_at,
                        };
                        synthetic_bars.push(nth_bar);
                    }
                }
                SymbolMode::NthMonth(2) => {
                    // Emit unadjusted bar if this is the L2 contract
                    if contract == l2 {
                        let nth_bar = LiveBar {
                            bar: flux_runtime::BarContext {
                                symbol: format!("{}=2", root),
                                open: bar.bar.open,
                                high: bar.bar.high,
                                low: bar.bar.low,
                                close: bar.bar.close,
                                volume: bar.bar.volume,
                                in_position: bar.bar.in_position,
                            },
                            connector_id: bar.connector_id.clone(),
                            received_at: bar.received_at,
                        };
                        synthetic_bars.push(nth_bar);
                    }
                }
                SymbolMode::NthMonth(n) => {
                    // For higher-order months, check if this contract is the Nth
                    // in the cycle starting from L1
                    let mut target = l1.clone();
                    for _ in 1..n {
                        target = QuarterlyCycle::next(&target);
                    }
                    if contract == target {
                        let nth_bar = LiveBar {
                            bar: flux_runtime::BarContext {
                                symbol: format!("{}={}", root, n),
                                open: bar.bar.open,
                                high: bar.bar.high,
                                low: bar.bar.low,
                                close: bar.bar.close,
                                volume: bar.bar.volume,
                                in_position: bar.bar.in_position,
                            },
                            connector_id: bar.connector_id.clone(),
                            received_at: bar.received_at,
                        };
                        synthetic_bars.push(nth_bar);
                    }
                }
            }
        }

        ProcessResult {
            bars: synthetic_bars,
            roll_event: None,
        }
    }

    /// Process a daily bar: combines bar routing, volume buffer push, and crossover evaluation.
    ///
    /// Unlike `process_bar` (designed for intraday data where volume accumulates across
    /// many bars within a session), this method treats each bar as a complete day:
    /// - Pushes the bar's volume directly into the rolling buffer (no intraday accumulator)
    /// - Evaluates crossover immediately after the push
    /// - Emits synthetic bars and triggers roll if crossover fires
    ///
    /// Use this for replay of daily-resampled data.
    pub fn process_daily_bar(&mut self, bar: &LiveBar, date: NaiveDate) -> ProcessResult {
        // Try to parse as a concrete contract
        let contract = match parse_concrete(&bar.bar.symbol) {
            Ok(c) => c,
            Err(_) => {
                return ProcessResult {
                    bars: vec![bar.clone()],
                    roll_event: None,
                };
            }
        };

        let product_root = &contract.root;
        if !self.state_machines.contains_key(product_root) {
            return ProcessResult {
                bars: vec![bar.clone()],
                roll_event: None,
            };
        }

        // Push volume directly into the appropriate buffer (skip intraday accumulator)
        // Also track the close price for ratio computation at roll time.
        let sm = self.state_machines.get_mut(product_root).unwrap();
        let volume = bar.bar.volume as u64;
        if contract == sm.l1 {
            sm.l1_volume_history.push(volume);
            sm.l1_last_close = bar.bar.close;
        } else if contract == sm.l2 {
            sm.l2_volume_history.push(volume);
            sm.l2_last_close = bar.bar.close;
        }

        // Evaluate crossover after each push
        let roll_event = if contract == sm.l2 || contract == sm.l1 {
            // Only evaluate after we have data for both — check if crossover fires
            let signal = sm.evaluate_crossover(date);
            if let Some(signal) = signal {
                // Capture close prices BEFORE execute_roll resets state
                let old_l1_close = sm.l1_last_close;
                let new_l1_close = sm.l2_last_close; // L2 becomes new L1

                let _transition = sm.execute_roll();
                let root = product_root.clone();

                // Apply the backward ratio adjustment if both prices are available
                let ratio = if old_l1_close > 0.0 && new_l1_close > 0.0 {
                    let old_contract_str = format_concrete(&signal.old_l1);
                    let new_contract_str = format_concrete(&signal.new_l1);
                    match self.adjusters.get_mut(&root) {
                        Some(adjuster) => {
                            adjuster.apply_roll(
                                old_l1_close,
                                new_l1_close,
                                date,
                                &old_contract_str,
                                &new_contract_str,
                            ).unwrap_or(1.0)
                        }
                        None => 1.0,
                    }
                } else {
                    1.0
                };

                self.roll_history.push(RollRecord {
                    date,
                    product_root: root.clone(),
                    old_contract: format_concrete(&signal.old_l1),
                    new_contract: format_concrete(&signal.new_l1),
                    adjustment_ratio: ratio,
                    position_rolled: false,
                });

                let has_continuous_sub = self.subscriptions.iter().any(|s| {
                    s.generic_symbol.root == root && s.generic_symbol.mode == SymbolMode::Continuous
                });

                if has_continuous_sub {
                    Some(RollEvent {
                        product_root: root,
                        old_contract: signal.old_l1,
                        new_contract: signal.new_l1,
                        position_qty: 0.0,
                        direction: Side::Buy,
                        adjustment_ratio: 1.0,
                        use_calendar_spread: true,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Build synthetic bars (same logic as process_bar)
        let sm = self.state_machines.get(product_root).unwrap();
        let l1 = sm.l1.clone();
        let l2 = sm.l2.clone();
        let root = product_root.clone();

        let mut synthetic_bars = Vec::new();
        for sub in &self.subscriptions {
            if sub.generic_symbol.root != root {
                continue;
            }
            match sub.generic_symbol.mode {
                SymbolMode::Continuous => {
                    if contract == l1 {
                        if let Some(adjuster) = self.adjusters.get(&root) {
                            let adjusted_bar = LiveBar {
                                bar: flux_runtime::BarContext {
                                    symbol: format!("{}=F", root),
                                    open: adjuster.adjust_price(bar.bar.open),
                                    high: adjuster.adjust_price(bar.bar.high),
                                    low: adjuster.adjust_price(bar.bar.low),
                                    close: adjuster.adjust_price(bar.bar.close),
                                    volume: adjuster.adjust_volume(bar.bar.volume as u64),
                                    in_position: bar.bar.in_position,
                                },
                                connector_id: bar.connector_id.clone(),
                                received_at: bar.received_at,
                            };
                            synthetic_bars.push(adjusted_bar);
                        }
                    }
                }
                SymbolMode::NthMonth(1) => {
                    if contract == l1 {
                        let nth_bar = LiveBar {
                            bar: flux_runtime::BarContext {
                                symbol: format!("{}=1", root),
                                ..bar.bar.clone()
                            },
                            connector_id: bar.connector_id.clone(),
                            received_at: bar.received_at,
                        };
                        synthetic_bars.push(nth_bar);
                    }
                }
                SymbolMode::NthMonth(2)
                    if contract == l2 => {
                        let nth_bar = LiveBar {
                            bar: flux_runtime::BarContext {
                                symbol: format!("{}=2", root),
                                ..bar.bar.clone()
                            },
                            connector_id: bar.connector_id.clone(),
                            received_at: bar.received_at,
                        };
                        synthetic_bars.push(nth_bar);
                    }
                _ => {}
            }
        }

        ProcessResult {
            bars: synthetic_bars,
            roll_event,
        }
    }

    /// Called at session close to evaluate crossover and potentially trigger a roll.
    ///
    /// The harness calls this once per product at the end of each trading session.
    /// If a crossover is detected, the roll is executed and a RollEvent may be emitted.
    pub fn end_of_session(&mut self, product_root: &str, date: NaiveDate) -> Option<RollEvent> {
        let sm = self.state_machines.get_mut(product_root)?;

        // Push intraday volumes into rolling buffers and reset accumulators
        sm.end_of_day();

        // Evaluate crossover condition
        let signal = sm.evaluate_crossover(date)?;

        // Crossover detected — execute the roll
        let _transition = sm.execute_roll();

        // Record roll in history (adjustment ratio populated later via apply_roll_adjustment)
        self.roll_history.push(RollRecord {
            date,
            product_root: product_root.to_string(),
            old_contract: format_concrete(&signal.old_l1),
            new_contract: format_concrete(&signal.new_l1),
            adjustment_ratio: 1.0, // Updated by harness after computing from close prices
            position_rolled: false, // Updated by harness after successful roll
        });

        // Emit RollEvent only if there's a =F subscription for this product
        let has_continuous_sub = self.subscriptions.iter().any(|s| {
            s.generic_symbol.root == product_root && s.generic_symbol.mode == SymbolMode::Continuous
        });

        if has_continuous_sub {
            Some(RollEvent {
                product_root: product_root.to_string(),
                old_contract: signal.old_l1,
                new_contract: signal.new_l1,
                position_qty: 0.0, // Filled in by harness from position tracker
                direction: Side::Buy, // Filled in by harness from position tracker
                adjustment_ratio: 1.0, // Updated after apply_roll_adjustment
                use_calendar_spread: true,
            })
        } else {
            None
        }
    }

    /// Apply a roll adjustment with known close prices.
    ///
    /// Called by the harness after a roll fires, supplying the actual closing
    /// prices for computing the adjustment ratio.
    pub fn apply_roll_adjustment(
        &mut self,
        product_root: &str,
        old_close: f64,
        new_close: f64,
        date: NaiveDate,
        old_contract: &str,
        new_contract: &str,
    ) -> Result<f64, RollError> {
        let adjuster = self
            .adjusters
            .get_mut(product_root)
            .ok_or_else(|| RollError::UnknownProduct(product_root.to_string()))?;
        let ratio = adjuster.apply_roll(old_close, new_close, date, old_contract, new_contract)?;

        // Update the most recent roll history entry with the computed ratio
        if let Some(last_record) = self.roll_history.last_mut() {
            if last_record.product_root == product_root && last_record.date == date {
                last_record.adjustment_ratio = ratio;
            }
        }

        Ok(ratio)
    }

    /// Register a generic symbol subscription from a strategy.
    ///
    /// If no state machine exists for this product root yet, creates one
    /// using the nearest non-expired contracts from the quarterly cycle.
    pub fn register_subscription(
        &mut self,
        generic_symbol: GenericSymbol,
        strategy_name: String,
    ) {
        let root = generic_symbol.root.clone();

        // Create state machine and adjuster if not yet registered
        if !self.state_machines.contains_key(&root) {
            let today = Utc::now().date_naive();
            let contracts = QuarterlyCycle::nearest_contracts(&root, today, 2);
            if contracts.len() >= 2 {
                let l1 = contracts[0].clone();
                let l2 = contracts[1].clone();
                let sm = RollStateMachine::new(root.clone(), l1, l2);
                self.state_machines.insert(root.clone(), sm);
                self.adjusters.insert(root.clone(), ContinuousAdjuster::new());
            }
        }

        // Record the subscription
        self.subscriptions.push(SymbolSubscription {
            strategy_name,
            generic_symbol,
        });
    }

    /// Register a subscription with explicit L1/L2 contracts (for testing or manual init).
    pub fn register_subscription_with_contracts(
        &mut self,
        generic_symbol: GenericSymbol,
        strategy_name: String,
        l1: ConcreteContract,
        l2: ConcreteContract,
    ) {
        let root = generic_symbol.root.clone();

        if !self.state_machines.contains_key(&root) {
            let sm = RollStateMachine::new(root.clone(), l1, l2);
            self.state_machines.insert(root.clone(), sm);
            self.adjusters.insert(root.clone(), ContinuousAdjuster::new());
        }

        self.subscriptions.push(SymbolSubscription {
            strategy_name,
            generic_symbol,
        });
    }

    /// Query the current contract mapping for a product root.
    ///
    /// Returns the current L1, L2, cumulative adjustment factor, and phase.
    pub fn current_mapping(&self, product_root: &str) -> Option<ContractMapping> {
        let sm = self.state_machines.get(product_root)?;
        let adjuster = self.adjusters.get(product_root)?;
        let (phase, l1, l2) = sm.current_state();

        Some(ContractMapping {
            product_root: product_root.to_string(),
            l1: l1.clone(),
            l2: l2.clone(),
            cumulative_adjustment_factor: adjuster.cumulative_factor,
            phase,
        })
    }

    /// Get the unadjusted (raw) price for order submission.
    ///
    /// Reverses the continuous adjustment for a product root.
    /// Returns the adjusted price unchanged if the product root is unknown.
    pub fn unadjusted_price(&self, product_root: &str, adjusted_price: f64) -> f64 {
        match self.adjusters.get(product_root) {
            Some(adjuster) => adjuster.unadjust_price(adjusted_price),
            None => adjusted_price,
        }
    }

    /// Get the concrete contracts that need to be subscribed on the connector
    /// for all registered product roots (L1 + L2 for each).
    pub fn required_subscriptions(&self) -> Vec<String> {
        let mut symbols = Vec::new();
        for sm in self.state_machines.values() {
            symbols.push(format_concrete(&sm.l1));
            symbols.push(format_concrete(&sm.l2));
        }
        symbols
    }

    /// Serialize the current state for persistence across restarts.
    ///
    /// Captures roll history, L1/L2 mappings, cumulative adjustment factors,
    /// and volume buffers into a `FuturesRollState` that can be stored and
    /// later restored via `restore_state`.
    pub fn snapshot_state(&self) -> FuturesRollState {
        let machines = self
            .state_machines
            .values()
            .map(|sm| SerializedRollMachine {
                product_root: sm.product_root.clone(),
                l1: format_concrete(&sm.l1),
                l2: format_concrete(&sm.l2),
                l1_volumes: sm.l1_volume_history.to_vec(),
                l2_volumes: sm.l2_volume_history.to_vec(),
                roll_latched: sm.roll_latched,
                phase: match sm.state {
                    RollPhase::Active => "active",
                    RollPhase::Rolling => "rolling",
                }
                .to_string(),
            })
            .collect();

        let adjusters = self
            .adjusters
            .iter()
            .map(|(root, adj)| SerializedAdjuster {
                product_root: root.clone(),
                cumulative_factor: adj.cumulative_factor,
                adjustments: adj.adjustments.clone(),
            })
            .collect();

        FuturesRollState {
            version: 1,
            machines,
            adjusters,
            roll_history: self.roll_history.clone(),
        }
    }

    /// Reconstruct state from a previously serialized `FuturesRollState`.
    ///
    /// Restores roll state machines (L1/L2 contracts, volume buffers, roll latch,
    /// phase), continuous adjusters (cumulative factor, adjustment history), and
    /// the complete roll history — without re-triggering historical rolls.
    ///
    /// Returns `Err(RollError::StateRestore)` if the state version is not supported,
    /// or if any serialized contract symbol fails to parse.
    pub fn restore_state(&mut self, state: &FuturesRollState) -> Result<(), RollError> {
        // Version check — graceful failure on mismatch
        if state.version != 1 {
            return Err(RollError::StateRestore(format!(
                "unsupported state version: {}, expected 1",
                state.version
            )));
        }

        // Restore state machines
        for sm_state in &state.machines {
            let l1 = parse_concrete(&sm_state.l1)
                .map_err(|e| RollError::StateRestore(e.to_string()))?;
            let l2 = parse_concrete(&sm_state.l2)
                .map_err(|e| RollError::StateRestore(e.to_string()))?;

            let mut sm = RollStateMachine::new(sm_state.product_root.clone(), l1, l2);
            sm.l1_volume_history = VolumeBuffer::from_vec(&sm_state.l1_volumes);
            sm.l2_volume_history = VolumeBuffer::from_vec(&sm_state.l2_volumes);
            sm.roll_latched = sm_state.roll_latched;
            sm.state = match sm_state.phase.as_str() {
                "rolling" => RollPhase::Rolling,
                _ => RollPhase::Active,
            };

            self.state_machines
                .insert(sm_state.product_root.clone(), sm);
        }

        // Restore adjusters
        for adj_state in &state.adjusters {
            let adj = ContinuousAdjuster {
                cumulative_factor: adj_state.cumulative_factor,
                adjustments: adj_state.adjustments.clone(),
                backward_mode: false,
            };
            self.adjusters
                .insert(adj_state.product_root.clone(), adj);
        }

        // Restore roll history
        self.roll_history = state.roll_history.clone();

        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── parse_generic tests ─────────────────────────────────────────────────

    #[test]
    fn parse_generic_continuous() {
        let result = parse_generic("ES=F").unwrap();
        assert_eq!(result.root, "ES");
        assert_eq!(result.mode, SymbolMode::Continuous);
    }

    #[test]
    fn parse_generic_nth_month() {
        let result = parse_generic("NQ=2").unwrap();
        assert_eq!(result.root, "NQ");
        assert_eq!(result.mode, SymbolMode::NthMonth(2));
    }

    #[test]
    fn parse_generic_single_char_root() {
        let result = parse_generic("A=1").unwrap();
        assert_eq!(result.root, "A");
        assert_eq!(result.mode, SymbolMode::NthMonth(1));
    }

    #[test]
    fn parse_generic_four_char_root() {
        let result = parse_generic("ABCD=F").unwrap();
        assert_eq!(result.root, "ABCD");
        assert_eq!(result.mode, SymbolMode::Continuous);
    }

    #[test]
    fn parse_generic_large_nth() {
        let result = parse_generic("ES=12").unwrap();
        assert_eq!(result.mode, SymbolMode::NthMonth(12));
    }

    #[test]
    fn parse_generic_rejects_empty_root() {
        assert!(parse_generic("=F").is_err());
    }

    #[test]
    fn parse_generic_rejects_five_char_root() {
        assert!(parse_generic("ABCDE=F").is_err());
    }

    #[test]
    fn parse_generic_rejects_lowercase_root() {
        assert!(parse_generic("es=F").is_err());
    }

    #[test]
    fn parse_generic_rejects_zero_mode() {
        assert!(parse_generic("ES=0").is_err());
    }

    #[test]
    fn parse_generic_rejects_invalid_mode() {
        assert!(parse_generic("ES=X").is_err());
    }

    #[test]
    fn parse_generic_rejects_no_equals() {
        assert!(parse_generic("ESF").is_err());
    }

    // ─── parse_concrete tests ────────────────────────────────────────────────

    #[test]
    fn parse_concrete_esh5() {
        let result = parse_concrete("ESH5").unwrap();
        assert_eq!(result.root, "ES");
        assert_eq!(result.month, MonthCode::H);
        assert_eq!(result.year, 5);
    }

    #[test]
    fn parse_concrete_nqm6() {
        let result = parse_concrete("NQM6").unwrap();
        assert_eq!(result.root, "NQ");
        assert_eq!(result.month, MonthCode::M);
        assert_eq!(result.year, 6);
    }

    #[test]
    fn parse_concrete_rtyu5() {
        let result = parse_concrete("RTYU5").unwrap();
        assert_eq!(result.root, "RTY");
        assert_eq!(result.month, MonthCode::U);
        assert_eq!(result.year, 5);
    }

    #[test]
    fn parse_concrete_ymz0() {
        let result = parse_concrete("YMZ0").unwrap();
        assert_eq!(result.root, "YM");
        assert_eq!(result.month, MonthCode::Z);
        assert_eq!(result.year, 0);
    }

    #[test]
    fn parse_concrete_single_char_root() {
        let result = parse_concrete("AH9").unwrap();
        assert_eq!(result.root, "A");
        assert_eq!(result.month, MonthCode::H);
        assert_eq!(result.year, 9);
    }

    #[test]
    fn parse_concrete_rejects_too_short() {
        assert!(parse_concrete("EH").is_err());
    }

    #[test]
    fn parse_concrete_rejects_invalid_month() {
        assert!(parse_concrete("ESA5").is_err());
    }

    #[test]
    fn parse_concrete_rejects_non_digit_year() {
        assert!(parse_concrete("ESHA").is_err());
    }

    #[test]
    fn parse_concrete_rejects_lowercase_root() {
        assert!(parse_concrete("esH5").is_err());
    }

    #[test]
    fn parse_concrete_rejects_five_char_root() {
        assert!(parse_concrete("ABCDEH5").is_err());
    }

    // ─── format tests ────────────────────────────────────────────────────────

    #[test]
    fn format_generic_continuous() {
        let sym = GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        };
        assert_eq!(format_generic(&sym), "ES=F");
    }

    #[test]
    fn format_generic_nth_month() {
        let sym = GenericSymbol {
            root: "NQ".to_string(),
            mode: SymbolMode::NthMonth(3),
        };
        assert_eq!(format_generic(&sym), "NQ=3");
    }

    #[test]
    fn format_concrete_esh5() {
        let contract = ConcreteContract {
            root: "ES".to_string(),
            month: MonthCode::H,
            year: 5,
        };
        assert_eq!(format_concrete(&contract), "ESH5");
    }

    #[test]
    fn format_concrete_rtyu9() {
        let contract = ConcreteContract {
            root: "RTY".to_string(),
            month: MonthCode::U,
            year: 9,
        };
        assert_eq!(format_concrete(&contract), "RTYU9");
    }

    // ─── parse_symbol dispatch tests ─────────────────────────────────────────

    #[test]
    fn parse_symbol_dispatches_to_generic() {
        let result = parse_symbol("ES=F").unwrap();
        assert!(matches!(result, SymbolKind::Generic(_)));
    }

    #[test]
    fn parse_symbol_dispatches_to_concrete() {
        let result = parse_symbol("ESH5").unwrap();
        assert!(matches!(result, SymbolKind::Concrete(_)));
    }

    // ─── round-trip tests ────────────────────────────────────────────────────

    #[test]
    fn generic_round_trip() {
        let sym = GenericSymbol {
            root: "ES".to_string(),
            mode: SymbolMode::Continuous,
        };
        let formatted = format_generic(&sym);
        let parsed = parse_generic(&formatted).unwrap();
        assert_eq!(parsed, sym);
    }

    #[test]
    fn concrete_round_trip() {
        let contract = ConcreteContract {
            root: "NQ".to_string(),
            month: MonthCode::M,
            year: 6,
        };
        let formatted = format_concrete(&contract);
        let parsed = parse_concrete(&formatted).unwrap();
        assert_eq!(parsed, contract);
    }

    // ─── VolumeBuffer tests ──────────────────────────────────────────────────

    #[test]
    fn volume_buffer_empty_returns_none() {
        let buf = VolumeBuffer::new();
        assert_eq!(buf.average(), None);
    }

    #[test]
    fn volume_buffer_under_five_returns_none() {
        let mut buf = VolumeBuffer::new();
        buf.push(100);
        assert_eq!(buf.average(), None);
        buf.push(200);
        assert_eq!(buf.average(), None);
        buf.push(300);
        assert_eq!(buf.average(), None);
        buf.push(400);
        assert_eq!(buf.average(), None);
    }

    #[test]
    fn volume_buffer_exactly_five_returns_average() {
        let mut buf = VolumeBuffer::new();
        buf.push(100);
        buf.push(200);
        buf.push(300);
        buf.push(400);
        buf.push(500);
        // Average = (100+200+300+400+500) / 5 = 300.0
        assert_eq!(buf.average(), Some(300.0));
    }

    #[test]
    fn volume_buffer_circular_overwrites() {
        let mut buf = VolumeBuffer::new();
        // Push 7 values; average should only use the last 5.
        buf.push(10);
        buf.push(20);
        buf.push(30);
        buf.push(40);
        buf.push(50);
        buf.push(60);
        buf.push(70);
        // Last 5 are: 30, 40, 50, 60, 70 → average = 250/5 = 50.0
        assert_eq!(buf.average(), Some(50.0));
    }

    #[test]
    fn volume_buffer_is_full() {
        let mut buf = VolumeBuffer::new();
        assert!(!buf.is_full());
        buf.push(1);
        assert!(!buf.is_full());
        buf.push(2);
        assert!(!buf.is_full());
        buf.push(3);
        assert!(!buf.is_full());
        buf.push(4);
        assert!(!buf.is_full());
        buf.push(5);
        assert!(buf.is_full());
    }

    // ─── QuarterlyCycle tests ────────────────────────────────────────────────

    #[test]
    fn quarterly_cycle_next_h_to_m() {
        let esh5 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let next = QuarterlyCycle::next(&esh5);
        assert_eq!(next.root, "ES");
        assert_eq!(next.month, MonthCode::M);
        assert_eq!(next.year, 5);
    }

    #[test]
    fn quarterly_cycle_next_m_to_u() {
        let esm5 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let next = QuarterlyCycle::next(&esm5);
        assert_eq!(next.root, "ES");
        assert_eq!(next.month, MonthCode::U);
        assert_eq!(next.year, 5);
    }

    #[test]
    fn quarterly_cycle_next_u_to_z() {
        let esu5 = ConcreteContract { root: "ES".to_string(), month: MonthCode::U, year: 5 };
        let next = QuarterlyCycle::next(&esu5);
        assert_eq!(next.root, "ES");
        assert_eq!(next.month, MonthCode::Z);
        assert_eq!(next.year, 5);
    }

    #[test]
    fn quarterly_cycle_next_z_to_h_year_wrap() {
        let esz5 = ConcreteContract { root: "ES".to_string(), month: MonthCode::Z, year: 5 };
        let next = QuarterlyCycle::next(&esz5);
        assert_eq!(next.root, "ES");
        assert_eq!(next.month, MonthCode::H);
        assert_eq!(next.year, 6);
    }

    #[test]
    fn quarterly_cycle_next_year_9_wraps_to_0() {
        let esz9 = ConcreteContract { root: "ES".to_string(), month: MonthCode::Z, year: 9 };
        let next = QuarterlyCycle::next(&esz9);
        assert_eq!(next.root, "ES");
        assert_eq!(next.month, MonthCode::H);
        assert_eq!(next.year, 0);
    }

    #[test]
    fn quarterly_cycle_previous_m_to_h() {
        let esm5 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let prev = QuarterlyCycle::previous(&esm5);
        assert_eq!(prev.root, "ES");
        assert_eq!(prev.month, MonthCode::H);
        assert_eq!(prev.year, 5);
    }

    #[test]
    fn quarterly_cycle_previous_h_to_z_year_wrap() {
        let esh5 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let prev = QuarterlyCycle::previous(&esh5);
        assert_eq!(prev.root, "ES");
        assert_eq!(prev.month, MonthCode::Z);
        assert_eq!(prev.year, 4);
    }

    #[test]
    fn quarterly_cycle_previous_year_0_wraps_to_9() {
        let esh0 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 0 };
        let prev = QuarterlyCycle::previous(&esh0);
        assert_eq!(prev.root, "ES");
        assert_eq!(prev.month, MonthCode::Z);
        assert_eq!(prev.year, 9);
    }

    #[test]
    fn quarterly_cycle_round_trip_next_then_previous() {
        let original = ConcreteContract { root: "NQ".to_string(), month: MonthCode::U, year: 7 };
        let result = QuarterlyCycle::previous(&QuarterlyCycle::next(&original));
        assert_eq!(result, original);
    }

    #[test]
    fn quarterly_cycle_round_trip_previous_then_next() {
        let original = ConcreteContract { root: "RTY".to_string(), month: MonthCode::Z, year: 3 };
        let result = QuarterlyCycle::next(&QuarterlyCycle::previous(&original));
        assert_eq!(result, original);
    }

    #[test]
    fn quarterly_cycle_nearest_contracts_march() {
        // In March 2025, H (March) is still active (we're in the expiry month).
        let today = chrono::NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
        let contracts = QuarterlyCycle::nearest_contracts("ES", today, 3);
        assert_eq!(contracts.len(), 3);
        // First should be ESH5 (March 2025 — still in month)
        assert_eq!(contracts[0].root, "ES");
        assert_eq!(contracts[0].month, MonthCode::H);
        assert_eq!(contracts[0].year, 5);
        // Second: ESM5
        assert_eq!(contracts[1].month, MonthCode::M);
        assert_eq!(contracts[1].year, 5);
        // Third: ESU5
        assert_eq!(contracts[2].month, MonthCode::U);
        assert_eq!(contracts[2].year, 5);
    }

    #[test]
    fn quarterly_cycle_nearest_contracts_april() {
        // In April 2025, H (March) is expired. First non-expired is M (June).
        let today = chrono::NaiveDate::from_ymd_opt(2025, 4, 10).unwrap();
        let contracts = QuarterlyCycle::nearest_contracts("ES", today, 3);
        assert_eq!(contracts.len(), 3);
        // First: ESM5 (June 2025)
        assert_eq!(contracts[0].root, "ES");
        assert_eq!(contracts[0].month, MonthCode::M);
        assert_eq!(contracts[0].year, 5);
        // Second: ESU5
        assert_eq!(contracts[1].month, MonthCode::U);
        assert_eq!(contracts[1].year, 5);
        // Third: ESZ5
        assert_eq!(contracts[2].month, MonthCode::Z);
        assert_eq!(contracts[2].year, 5);
    }

    #[test]
    fn quarterly_cycle_nearest_contracts_december() {
        // In December 2025, Z (December) is still active (we're in the expiry month).
        let today = chrono::NaiveDate::from_ymd_opt(2025, 12, 5).unwrap();
        let contracts = QuarterlyCycle::nearest_contracts("ES", today, 3);
        assert_eq!(contracts.len(), 3);
        // First: ESZ5 (December 2025 — still in month)
        assert_eq!(contracts[0].root, "ES");
        assert_eq!(contracts[0].month, MonthCode::Z);
        assert_eq!(contracts[0].year, 5);
        // Second: ESH6
        assert_eq!(contracts[1].month, MonthCode::H);
        assert_eq!(contracts[1].year, 6);
        // Third: ESM6
        assert_eq!(contracts[2].month, MonthCode::M);
        assert_eq!(contracts[2].year, 6);
    }

    // ─── ContinuousAdjuster tests ───────────────────────────────────────────

    #[test]
    fn adjuster_new_has_factor_one() {
        let adj = ContinuousAdjuster::new();
        assert_eq!(adj.cumulative_factor, 1.0);
        assert!(adj.adjustments.is_empty());
    }

    #[test]
    fn adjuster_apply_roll_basic() {
        let mut adj = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        let ratio = adj.apply_roll(100.0, 102.0, date, "ESH5", "ESM5").unwrap();

        assert!((ratio - 1.02).abs() < 1e-10);
        assert!((adj.cumulative_factor - 1.02).abs() < 1e-10);
        assert_eq!(adj.adjustments.len(), 1);
        assert_eq!(adj.adjustments[0].old_contract, "ESH5");
        assert_eq!(adj.adjustments[0].new_contract, "ESM5");
        assert!((adj.adjustments[0].ratio - 1.02).abs() < 1e-10);
        assert!((adj.adjustments[0].cumulative_factor_after - 1.02).abs() < 1e-10);
    }

    #[test]
    fn adjuster_apply_roll_multiple() {
        let mut adj = ContinuousAdjuster::new();
        let d1 = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2025, 6, 13).unwrap();

        adj.apply_roll(100.0, 102.0, d1, "ESH5", "ESM5").unwrap();
        adj.apply_roll(102.0, 105.0, d2, "ESM5", "ESU5").unwrap();

        // Expected: 1.02 * (105/102)
        let expected = 1.02 * (105.0 / 102.0);
        assert!((adj.cumulative_factor - expected).abs() < 1e-10);
        assert_eq!(adj.adjustments.len(), 2);
    }

    #[test]
    fn adjuster_apply_roll_division_by_zero() {
        let mut adj = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        let result = adj.apply_roll(0.0, 102.0, date, "ESH5", "ESM5");

        assert!(result.is_err());
        // Factor should remain unchanged
        assert_eq!(adj.cumulative_factor, 1.0);
        assert!(adj.adjustments.is_empty());
    }

    #[test]
    fn adjuster_adjust_price() {
        let mut adj = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        adj.apply_roll(100.0, 102.0, date, "ESH5", "ESM5").unwrap();

        let adjusted = adj.adjust_price(5000.0);
        assert!((adjusted - 5000.0 * 1.02).abs() < 1e-10);
    }

    #[test]
    fn adjuster_unadjust_price() {
        let mut adj = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        adj.apply_roll(100.0, 102.0, date, "ESH5", "ESM5").unwrap();

        let adjusted = adj.adjust_price(5000.0);
        let unadjusted = adj.unadjust_price(adjusted);
        assert!((unadjusted - 5000.0).abs() < 1e-10);
    }

    #[test]
    fn adjuster_price_round_trip() {
        let mut adj = ContinuousAdjuster::new();
        let d1 = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2025, 6, 13).unwrap();
        adj.apply_roll(100.0, 102.0, d1, "ESH5", "ESM5").unwrap();
        adj.apply_roll(102.0, 99.0, d2, "ESM5", "ESU5").unwrap();

        let raw_price = 4850.25;
        let adjusted = adj.adjust_price(raw_price);
        let recovered = adj.unadjust_price(adjusted);
        assert!((recovered - raw_price).abs() < 1e-10);
    }

    #[test]
    fn adjuster_adjust_volume() {
        let mut adj = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        adj.apply_roll(100.0, 102.0, date, "ESH5", "ESM5").unwrap();

        let adjusted_vol = adj.adjust_volume(10000);
        let expected = 10000.0 / 1.02;
        assert!((adjusted_vol - expected).abs() < 1e-10);
    }

    #[test]
    fn adjuster_no_adjustment_passthrough() {
        let adj = ContinuousAdjuster::new();
        // With factor = 1.0, prices should pass through unchanged
        assert_eq!(adj.adjust_price(123.45), 123.45);
        assert_eq!(adj.unadjust_price(123.45), 123.45);
        assert_eq!(adj.adjust_volume(5000), 5000.0);
    }

    // ─── RollStateMachine tests ──────────────────────────────────────────────

    #[test]
    fn roll_state_machine_new() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        let (phase, current_l1, current_l2) = sm.current_state();
        assert_eq!(phase, RollPhase::Active);
        assert_eq!(current_l1, &l1);
        assert_eq!(current_l2, &l2);
        assert!(!sm.roll_latched);
    }

    #[test]
    fn accumulate_volume_for_l1() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        sm.accumulate_volume(&l1, 100);
        sm.accumulate_volume(&l1, 200);
        assert_eq!(sm.l1_intraday_volume, 300);
        assert_eq!(sm.l2_intraday_volume, 0);
    }

    #[test]
    fn accumulate_volume_for_l2() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        sm.accumulate_volume(&l2, 500);
        assert_eq!(sm.l2_intraday_volume, 500);
        assert_eq!(sm.l1_intraday_volume, 0);
    }

    #[test]
    fn accumulate_volume_ignores_other_contracts() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let other = ConcreteContract { root: "ES".to_string(), month: MonthCode::U, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        sm.accumulate_volume(&other, 999);
        assert_eq!(sm.l1_intraday_volume, 0);
        assert_eq!(sm.l2_intraday_volume, 0);
    }

    #[test]
    fn end_of_day_pushes_and_resets() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        sm.accumulate_volume(&l1, 1000);
        sm.accumulate_volume(&l2, 2000);
        sm.end_of_day();

        assert_eq!(sm.l1_intraday_volume, 0);
        assert_eq!(sm.l2_intraday_volume, 0);
        // Buffers not yet full (only 1 day)
        assert!(!sm.l1_volume_history.is_full());
    }

    #[test]
    fn evaluate_crossover_returns_none_when_buffers_not_full() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        // Only 3 days of data
        for _ in 0..3 {
            sm.accumulate_volume(&l1, 100);
            sm.accumulate_volume(&l2, 200);
            sm.end_of_day();
        }

        let date = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        assert!(sm.evaluate_crossover(date).is_none());
    }

    #[test]
    fn evaluate_crossover_returns_none_when_latched() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());
        sm.roll_latched = true;

        // Fill buffers with L2 > L1
        for _ in 0..5 {
            sm.accumulate_volume(&l1, 100);
            sm.accumulate_volume(&l2, 200);
            sm.end_of_day();
        }

        let date = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        assert!(sm.evaluate_crossover(date).is_none());
    }

    #[test]
    fn evaluate_crossover_returns_none_when_l1_greater() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        // Fill buffers with L1 > L2
        for _ in 0..5 {
            sm.accumulate_volume(&l1, 500);
            sm.accumulate_volume(&l2, 100);
            sm.end_of_day();
        }

        let date = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        assert!(sm.evaluate_crossover(date).is_none());
    }

    #[test]
    fn evaluate_crossover_fires_when_l2_exceeds_l1() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        // Fill buffers with L2 > L1
        for _ in 0..5 {
            sm.accumulate_volume(&l1, 100);
            sm.accumulate_volume(&l2, 200);
            sm.end_of_day();
        }

        let date = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        let signal = sm.evaluate_crossover(date).expect("should emit RollSignal");
        assert_eq!(signal.product_root, "ES");
        assert_eq!(signal.old_l1, l1);
        assert_eq!(signal.new_l1, l2);
        assert_eq!(signal.l1_avg_volume, 100.0);
        assert_eq!(signal.l2_avg_volume, 200.0);
        assert_eq!(signal.trigger_date, date);
    }

    #[test]
    fn execute_roll_promotes_contracts() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        let transition = sm.execute_roll();

        assert_eq!(transition.old_l1, l1);
        assert_eq!(transition.new_l1, l2);
        // new_l2 should be next after ESM5, which is ESU5
        let expected_new_l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::U, year: 5 };
        assert_eq!(transition.new_l2, expected_new_l2);

        // State machine should be back to Active with updated contracts
        let (phase, current_l1, current_l2) = sm.current_state();
        assert_eq!(phase, RollPhase::Active);
        assert_eq!(current_l1, &l2);
        assert_eq!(current_l2, &expected_new_l2);
        assert!(sm.roll_latched);
    }

    #[test]
    fn execute_roll_resets_volume_tracking() {
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        // Accumulate some volume
        for _ in 0..5 {
            sm.accumulate_volume(&l1, 100);
            sm.accumulate_volume(&l2, 200);
            sm.end_of_day();
        }
        sm.accumulate_volume(&l1, 50);

        sm.execute_roll();

        // All volume tracking should be reset
        assert_eq!(sm.l1_intraday_volume, 0);
        assert_eq!(sm.l2_intraday_volume, 0);
        assert!(!sm.l1_volume_history.is_full());
        assert!(!sm.l2_volume_history.is_full());
    }

    #[test]
    fn execute_roll_year_wrap() {
        // Test roll from ESU5/ESZ5 → new L2 should be ESH6
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::U, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::Z, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1.clone(), l2.clone());

        let transition = sm.execute_roll();

        assert_eq!(transition.new_l1, l2);
        // new L2 after ESZ5 should be ESH6
        let expected_new_l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::H, year: 6 };
        assert_eq!(transition.new_l2, expected_new_l2);
    }

    #[test]
    fn full_roll_lifecycle() {
        let l1 = ConcreteContract { root: "NQ".to_string(), month: MonthCode::H, year: 5 };
        let l2 = ConcreteContract { root: "NQ".to_string(), month: MonthCode::M, year: 5 };
        let mut sm = RollStateMachine::new("NQ".to_string(), l1.clone(), l2.clone());

        let date = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();

        // Day 1-4: L1 has more volume (no crossover yet - buffer not full)
        for _ in 0..4 {
            sm.accumulate_volume(&l1, 50000);
            sm.accumulate_volume(&l2, 30000);
            sm.end_of_day();
        }
        assert!(sm.evaluate_crossover(date).is_none());

        // Day 5: buffer is now full but L1 > L2 → no signal
        sm.accumulate_volume(&l1, 50000);
        sm.accumulate_volume(&l2, 30000);
        sm.end_of_day();
        assert!(sm.evaluate_crossover(date).is_none());

        // Now fill with L2 > L1 for 5 fresh days (replace old data)
        for _ in 0..5 {
            sm.accumulate_volume(&l1, 20000);
            sm.accumulate_volume(&l2, 60000);
            sm.end_of_day();
        }

        // Now crossover should fire
        let signal = sm.evaluate_crossover(date).expect("crossover should fire");
        assert_eq!(signal.old_l1.month, MonthCode::H);
        assert_eq!(signal.new_l1.month, MonthCode::M);

        // Execute the roll
        let transition = sm.execute_roll();
        assert_eq!(transition.old_l1.month, MonthCode::H);
        assert_eq!(transition.new_l1.month, MonthCode::M);
        assert_eq!(transition.new_l2.month, MonthCode::U);

        // After roll, latched — no further crossover even with L2 > L1
        let new_l1 = sm.l1.clone();
        let new_l2 = sm.l2.clone();
        for _ in 0..5 {
            sm.accumulate_volume(&new_l1, 10000);
            sm.accumulate_volume(&new_l2, 90000);
            sm.end_of_day();
        }
        assert!(sm.evaluate_crossover(date).is_none());
    }

    // ─── VolumeBuffer to_vec / from_vec tests ────────────────────────────────

    #[test]
    fn volume_buffer_to_vec_empty() {
        let buf = VolumeBuffer::new();
        assert_eq!(buf.to_vec(), Vec::<u64>::new());
    }

    #[test]
    fn volume_buffer_to_vec_partial() {
        let mut buf = VolumeBuffer::new();
        buf.push(10);
        buf.push(20);
        buf.push(30);
        assert_eq!(buf.to_vec(), vec![10, 20, 30]);
    }

    #[test]
    fn volume_buffer_to_vec_full() {
        let mut buf = VolumeBuffer::new();
        buf.push(100);
        buf.push(200);
        buf.push(300);
        buf.push(400);
        buf.push(500);
        assert_eq!(buf.to_vec(), vec![100, 200, 300, 400, 500]);
    }

    #[test]
    fn volume_buffer_to_vec_circular_wrap() {
        let mut buf = VolumeBuffer::new();
        // Push 7 values; buffer keeps last 5 in order
        buf.push(10);
        buf.push(20);
        buf.push(30);
        buf.push(40);
        buf.push(50);
        buf.push(60);
        buf.push(70);
        // Oldest-first should be: 30, 40, 50, 60, 70
        assert_eq!(buf.to_vec(), vec![30, 40, 50, 60, 70]);
    }

    #[test]
    fn volume_buffer_from_vec_round_trip() {
        let mut original = VolumeBuffer::new();
        original.push(100);
        original.push(200);
        original.push(300);
        original.push(400);
        original.push(500);

        let serialized = original.to_vec();
        let restored = VolumeBuffer::from_vec(&serialized);

        assert_eq!(restored.average(), original.average());
        assert_eq!(restored.to_vec(), original.to_vec());
    }

    #[test]
    fn volume_buffer_from_vec_partial_round_trip() {
        let mut original = VolumeBuffer::new();
        original.push(42);
        original.push(99);

        let serialized = original.to_vec();
        let restored = VolumeBuffer::from_vec(&serialized);

        assert_eq!(restored.average(), None); // not full
        assert_eq!(restored.to_vec(), vec![42, 99]);
    }

    #[test]
    fn volume_buffer_from_vec_empty() {
        let restored = VolumeBuffer::from_vec(&[]);
        assert_eq!(restored.average(), None);
        assert_eq!(restored.to_vec(), Vec::<u64>::new());
    }

    // ─── snapshot_state / restore_state tests ────────────────────────────────

    /// Helper to create an empty FuturesRollManager for testing.
    fn test_manager() -> FuturesRollManager {
        let registry = Arc::new(ProductRegistry::from_entries(&[]));
        let calendar_toml = r#"
[[session]]
exchange = "CME"
open = "08:30"
close = "15:15"
timezone = "America/Chicago"
"#;
        let calendar = Arc::new(MarketCalendar::from_toml(calendar_toml).unwrap());
        FuturesRollManager {
            state_machines: HashMap::new(),
            adjusters: HashMap::new(),
            subscriptions: Vec::new(),
            product_registry: registry,
            calendar,
            roll_history: Vec::new(),
        }
    }

    #[test]
    fn snapshot_state_basic() {
        let mut manager = test_manager();

        // Add a state machine
        let l1 = ConcreteContract { root: "ES".to_string(), month: MonthCode::M, year: 5 };
        let l2 = ConcreteContract { root: "ES".to_string(), month: MonthCode::U, year: 5 };
        let mut sm = RollStateMachine::new("ES".to_string(), l1, l2);
        sm.roll_latched = true;
        for _ in 0..5 {
            sm.l1_volume_history.push(1000);
            sm.l2_volume_history.push(2000);
        }
        manager.state_machines.insert("ES".to_string(), sm);

        // Add an adjuster
        let mut adj = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        adj.apply_roll(100.0, 102.0, date, "ESH5", "ESM5").unwrap();
        manager.adjusters.insert("ES".to_string(), adj);

        // Add a roll record
        manager.roll_history.push(RollRecord {
            date,
            product_root: "ES".to_string(),
            old_contract: "ESH5".to_string(),
            new_contract: "ESM5".to_string(),
            adjustment_ratio: 1.02,
            position_rolled: true,
        });

        let state = manager.snapshot_state();

        assert_eq!(state.version, 1);
        assert_eq!(state.machines.len(), 1);
        assert_eq!(state.machines[0].product_root, "ES");
        assert_eq!(state.machines[0].l1, "ESM5");
        assert_eq!(state.machines[0].l2, "ESU5");
        assert!(state.machines[0].roll_latched);
        assert_eq!(state.machines[0].phase, "active");
        assert_eq!(state.machines[0].l1_volumes, vec![1000; 5]);
        assert_eq!(state.machines[0].l2_volumes, vec![2000; 5]);
        assert_eq!(state.adjusters.len(), 1);
        assert_eq!(state.adjusters[0].product_root, "ES");
        assert!((state.adjusters[0].cumulative_factor - 1.02).abs() < 1e-10);
        assert_eq!(state.adjusters[0].adjustments.len(), 1);
        assert_eq!(state.roll_history.len(), 1);
    }

    #[test]
    fn restore_state_basic() {
        let mut manager = test_manager();

        let date = NaiveDate::from_ymd_opt(2025, 3, 14).unwrap();
        let state = FuturesRollState {
            version: 1,
            machines: vec![SerializedRollMachine {
                product_root: "ES".to_string(),
                l1: "ESM5".to_string(),
                l2: "ESU5".to_string(),
                l1_volumes: vec![1000, 2000, 3000, 4000, 5000],
                l2_volumes: vec![6000, 7000, 8000, 9000, 10000],
                roll_latched: true,
                phase: "active".to_string(),
            }],
            adjusters: vec![SerializedAdjuster {
                product_root: "ES".to_string(),
                cumulative_factor: 1.02,
                adjustments: vec![AdjustmentRecord {
                    date,
                    old_contract: "ESH5".to_string(),
                    new_contract: "ESM5".to_string(),
                    ratio: 1.02,
                    cumulative_factor_after: 1.02,
                }],
            }],
            roll_history: vec![RollRecord {
                date,
                product_root: "ES".to_string(),
                old_contract: "ESH5".to_string(),
                new_contract: "ESM5".to_string(),
                adjustment_ratio: 1.02,
                position_rolled: true,
            }],
        };

        manager.restore_state(&state).unwrap();

        // Verify state machine
        let sm = manager.state_machines.get("ES").unwrap();
        assert_eq!(format_concrete(&sm.l1), "ESM5");
        assert_eq!(format_concrete(&sm.l2), "ESU5");
        assert!(sm.roll_latched);
        assert_eq!(sm.state, RollPhase::Active);
        assert!(sm.l1_volume_history.is_full());
        assert!(sm.l2_volume_history.is_full());
        assert_eq!(sm.l1_volume_history.average(), Some(3000.0));
        assert_eq!(sm.l2_volume_history.average(), Some(8000.0));

        // Verify adjuster
        let adj = manager.adjusters.get("ES").unwrap();
        assert!((adj.cumulative_factor - 1.02).abs() < 1e-10);
        assert_eq!(adj.adjustments.len(), 1);

        // Verify roll history
        assert_eq!(manager.roll_history.len(), 1);
        assert_eq!(manager.roll_history[0].product_root, "ES");
    }

    #[test]
    fn restore_state_version_mismatch() {
        let mut manager = test_manager();

        let state = FuturesRollState {
            version: 99,
            machines: Vec::new(),
            adjusters: Vec::new(),
            roll_history: Vec::new(),
        };

        let result = manager.restore_state(&state);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unsupported state version: 99"));
    }

    #[test]
    fn restore_state_invalid_contract_symbol() {
        let mut manager = test_manager();

        let state = FuturesRollState {
            version: 1,
            machines: vec![SerializedRollMachine {
                product_root: "ES".to_string(),
                l1: "INVALID".to_string(),
                l2: "ESU5".to_string(),
                l1_volumes: Vec::new(),
                l2_volumes: Vec::new(),
                roll_latched: false,
                phase: "active".to_string(),
            }],
            adjusters: Vec::new(),
            roll_history: Vec::new(),
        };

        let result = manager.restore_state(&state);
        assert!(result.is_err());
    }

    #[test]
    fn snapshot_restore_round_trip() {
        let mut manager = test_manager();

        // Set up state
        let l1 = ConcreteContract { root: "NQ".to_string(), month: MonthCode::U, year: 5 };
        let l2 = ConcreteContract { root: "NQ".to_string(), month: MonthCode::Z, year: 5 };
        let mut sm = RollStateMachine::new("NQ".to_string(), l1, l2);
        sm.l1_volume_history.push(500);
        sm.l1_volume_history.push(600);
        sm.l2_volume_history.push(700);
        sm.l2_volume_history.push(800);
        manager.state_machines.insert("NQ".to_string(), sm);

        let mut adj = ContinuousAdjuster::new();
        let date = NaiveDate::from_ymd_opt(2025, 6, 13).unwrap();
        adj.apply_roll(200.0, 198.0, date, "NQM5", "NQU5").unwrap();
        manager.adjusters.insert("NQ".to_string(), adj);

        // Snapshot
        let state = manager.snapshot_state();

        // Restore into fresh manager
        let mut restored_manager = test_manager();
        restored_manager.restore_state(&state).unwrap();

        // Verify equivalence
        let sm = restored_manager.state_machines.get("NQ").unwrap();
        assert_eq!(format_concrete(&sm.l1), "NQU5");
        assert_eq!(format_concrete(&sm.l2), "NQZ5");
        assert_eq!(sm.l1_volume_history.to_vec(), vec![500, 600]);
        assert_eq!(sm.l2_volume_history.to_vec(), vec![700, 800]);

        let adj = restored_manager.adjusters.get("NQ").unwrap();
        assert!((adj.cumulative_factor - (198.0 / 200.0)).abs() < 1e-10);
        assert_eq!(adj.adjustments.len(), 1);
    }

    #[test]
    fn restore_state_rolling_phase() {
        let mut manager = test_manager();

        let state = FuturesRollState {
            version: 1,
            machines: vec![SerializedRollMachine {
                product_root: "ES".to_string(),
                l1: "ESM5".to_string(),
                l2: "ESU5".to_string(),
                l1_volumes: Vec::new(),
                l2_volumes: Vec::new(),
                roll_latched: false,
                phase: "rolling".to_string(),
            }],
            adjusters: Vec::new(),
            roll_history: Vec::new(),
        };

        manager.restore_state(&state).unwrap();

        let sm = manager.state_machines.get("ES").unwrap();
        assert_eq!(sm.state, RollPhase::Rolling);
        assert!(!sm.roll_latched);
    }
}
