use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    pub(crate) static INDICATOR_STATE: RefCell<HashMap<String, IndicatorState>> =
        RefCell::new(HashMap::new());
}

/// Reset all indicator state. Called at the start of a new backtest run.
pub fn reset_indicator_state() {
    INDICATOR_STATE.with(|state| {
        state.borrow_mut().clear();
    });
}

/// The internal state variants for each indicator type.
pub(crate) enum IndicatorState {
    Sma(SmaState),
    Ema(EmaState),
}

/// Rolling buffer state for SMA computation.
pub(crate) struct SmaState {
    buffer: Vec<f64>,
    period: usize,
    index: usize,
    count: usize,
    sum: f64,
}

impl SmaState {
    pub fn new(period: usize) -> Self {
        Self {
            buffer: vec![0.0; period],
            period,
            index: 0,
            count: 0,
            sum: 0.0,
        }
    }

    pub fn next(&mut self, value: f64) -> f64 {
        if self.count < self.period {
            // Still filling the buffer
            self.buffer[self.index] = value;
            self.sum += value;
            self.count += 1;
            self.index = (self.index + 1) % self.period;
            self.sum / self.count as f64
        } else {
            // Buffer is full — subtract oldest, add newest
            self.sum -= self.buffer[self.index];
            self.buffer[self.index] = value;
            self.sum += value;
            self.index = (self.index + 1) % self.period;
            self.sum / self.period as f64
        }
    }
}

/// EMA state: stores the previous EMA value and smoothing factor.
pub(crate) struct EmaState {
    prev_ema: Option<f64>,
    k: f64,
}

impl EmaState {
    pub fn new(period: usize) -> Self {
        Self {
            prev_ema: None,
            k: 2.0 / (period as f64 + 1.0),
        }
    }

    pub fn next(&mut self, value: f64) -> f64 {
        let ema = match self.prev_ema {
            None => value,
            Some(prev) => value * self.k + prev * (1.0 - self.k),
        };
        self.prev_ema = Some(ema);
        ema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SmaState tests ---

    #[test]
    fn sma_filling_phase_single_value() {
        let mut sma = SmaState::new(3);
        let result = sma.next(10.0);
        assert_eq!(result, 10.0);
    }

    #[test]
    fn sma_filling_phase_partial() {
        let mut sma = SmaState::new(3);
        sma.next(10.0);
        let result = sma.next(20.0);
        // Mean of [10, 20] = 15
        assert_eq!(result, 15.0);
    }

    #[test]
    fn sma_filling_phase_complete() {
        let mut sma = SmaState::new(3);
        sma.next(10.0);
        sma.next(20.0);
        let result = sma.next(30.0);
        // Mean of [10, 20, 30] = 20
        assert_eq!(result, 20.0);
    }

    #[test]
    fn sma_full_phase() {
        let mut sma = SmaState::new(3);
        sma.next(10.0);
        sma.next(20.0);
        sma.next(30.0);
        let result = sma.next(40.0);
        // Mean of [20, 30, 40] = 30
        assert_eq!(result, 30.0);
    }

    #[test]
    fn sma_wraparound() {
        let mut sma = SmaState::new(3);
        sma.next(1.0);
        sma.next(2.0);
        sma.next(3.0);
        sma.next(4.0);
        sma.next(5.0);
        let result = sma.next(6.0);
        // Mean of [4, 5, 6] = 5
        assert_eq!(result, 5.0);
    }

    #[test]
    fn sma_period_one() {
        let mut sma = SmaState::new(1);
        assert_eq!(sma.next(5.0), 5.0);
        assert_eq!(sma.next(10.0), 10.0);
        assert_eq!(sma.next(3.0), 3.0);
    }

    // --- EmaState tests ---

    #[test]
    fn ema_first_value_returned() {
        let mut ema = EmaState::new(3);
        let result = ema.next(10.0);
        assert_eq!(result, 10.0);
    }

    #[test]
    fn ema_subsequent_values() {
        let mut ema = EmaState::new(3);
        ema.next(10.0);
        let result = ema.next(20.0);
        // k = 2.0 / (3 + 1) = 0.5
        // EMA = 20 * 0.5 + 10 * 0.5 = 15
        assert_eq!(result, 15.0);
    }

    #[test]
    fn ema_smoothing_factor() {
        let ema = EmaState::new(9);
        // k = 2.0 / (9 + 1) = 0.2
        assert!((ema.k - 0.2).abs() < 1e-10);
    }

    #[test]
    fn ema_multiple_values() {
        let mut ema = EmaState::new(3);
        // k = 0.5
        let r1 = ema.next(10.0); // 10.0
        assert_eq!(r1, 10.0);
        let r2 = ema.next(20.0); // 20*0.5 + 10*0.5 = 15
        assert_eq!(r2, 15.0);
        let r3 = ema.next(30.0); // 30*0.5 + 15*0.5 = 22.5
        assert_eq!(r3, 22.5);
    }

    #[test]
    fn ema_period_one() {
        let mut ema = EmaState::new(1);
        // k = 2.0 / (1 + 1) = 1.0
        // EMA always equals the latest value
        assert_eq!(ema.next(5.0), 5.0);
        assert_eq!(ema.next(10.0), 10.0);
        assert_eq!(ema.next(3.0), 3.0);
    }

    // --- reset_indicator_state tests ---

    #[test]
    fn reset_clears_state() {
        INDICATOR_STATE.with(|state| {
            state
                .borrow_mut()
                .insert("test_key".to_string(), IndicatorState::Sma(SmaState::new(3)));
        });

        reset_indicator_state();

        INDICATOR_STATE.with(|state| {
            assert!(state.borrow().is_empty());
        });
    }
}
