use crate::algs::RunningAverage;

const SMOOTHING_SAMPLES: usize = 10;
pub const SLOW_LATENCY_MS: f64 = 10.0;
pub const EMERGENCY_LATENCY_MS: f64 = 1000.0;

/// Rusty's latched bandwidth decision. Time is monotonic elapsed seconds.
pub struct LatencyControl {
    fast: bool,
    average: RunningAverage,
    samples: usize,
    previous: Option<(f64, f64)>,
    pub rolling_latency_ms: Option<f64>,
}

impl LatencyControl {
    pub fn new() -> Self {
        Self {
            fast: true,
            average: RunningAverage::new(SMOOTHING_SAMPLES),
            samples: 0,
            previous: None,
            rolling_latency_ms: None,
        }
    }

    fn reset_trend(&mut self) {
        self.average.clear();
        self.samples = 0;
        self.previous = None;
        self.rolling_latency_ms = None;
    }

    pub fn sample(&mut self, seconds: f64, latency: Option<f64>) -> (bool, Option<f64>) {
        let Some(latency) = latency.filter(|v| v.is_finite() && *v >= 0.0) else {
            // Do not bridge an unknown interval with an apparently valid slope.
            // Discard trend history, but preserve the latched bandwidth state.
            self.reset_trend();
            return (self.fast, None);
        };
        let smoothed = self.average.push(latency);
        self.rolling_latency_ms = Some(smoothed);
        // Both limits apply immediately, even with a partial window and no slope.
        // Recovery uses the same rolling window so one low reading cannot clear
        // accumulated latency. Missing data preserves the latched state above.
        self.fast = latency < EMERGENCY_LATENCY_MS && smoothed < SLOW_LATENCY_MS;
        self.samples = (self.samples + 1).min(SMOOTHING_SAMPLES);
        if self.samples < SMOOTHING_SAMPLES {
            return (self.fast, None);
        }
        // Compare full moving averages only; a partially filled window has
        // different weighting. At 100 ms sampling, ten samples span about 1 s.
        let slope = self.previous.and_then(|(time, value)| {
            (seconds > time).then(|| (smoothed - value) / (seconds - time))
        });
        self.previous = Some((seconds, smoothed));
        // Slope remains an early-warning diagnostic, never a trigger gate.
        (self.fast, slope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_threshold_is_immediate_including_partial_windows() {
        let mut control = LatencyControl::new();
        assert!(control.sample(0.0, Some(9.0)).0);
        assert_eq!(control.sample(0.1, Some(11.0)), (false, None));
        assert_eq!(control.rolling_latency_ms, Some(10.0));
        let mut fresh = LatencyControl::new();
        assert_eq!(fresh.sample(0.0, Some(10.0)), (false, None));
    }

    #[test]
    fn flat_and_falling_averages_still_force_slow() {
        for rate in [0.0, -1.0] {
            let mut control = LatencyControl::new();
            for tick in 0..20 {
                assert!(!control.sample(tick as f64, Some(30.0 + rate * tick as f64)).0);
            }
            assert!(control.sample(20.0, Some(30.0 + rate * 20.0)).1.unwrap() <= 0.0);
        }
    }

    #[test]
    fn emergency_is_immediate_without_slope_or_consecutive_samples() {
        let mut control = LatencyControl::new();
        for tick in 0..10 {
            assert!(control.sample(tick as f64, Some(0.0)).0);
        }
        // Repeated time deliberately makes slope unavailable.
        assert_eq!(control.sample(9.0, Some(1000.0)), (false, None));
        assert!(!control.sample(10.0, Some(0.0)).0);
        for tick in 11..20 { control.sample(tick as f64, Some(0.0)); }
        assert!(control.fast);
    }

    #[test]
    fn emergency_is_independent_of_average_window_size() {
        let mut control = LatencyControl::new();
        // Exercise the emergency guard independently, even if smoothing is
        // lengthened enough in the future to dilute a single emergency sample.
        control.average = RunningAverage::new(1000);
        for tick in 0..1000 { control.sample(tick as f64, Some(0.0)); }
        assert!(!control.sample(999.0, Some(1000.0)).0);
        assert_eq!(control.rolling_latency_ms, Some(1.0));
    }

    #[test]
    fn missing_or_invalid_latency_preserves_slow_and_resets_trend() {
        for invalid in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-1.0)] {
            let mut control = LatencyControl::new();
            assert!(!control.sample(0.0, Some(1000.0)).0);
            assert_eq!(control.sample(1.0, invalid), (false, None));
            assert_eq!(control.rolling_latency_ms, None);
            assert!(!control.sample(2.0, Some(10.0)).0);
            assert!(control.sample(3.0, Some(0.0)).0);
        }
    }

    #[test]
    fn slope_warns_below_threshold_without_forcing_slow() {
        let mut control = LatencyControl::new();
        for tick in 0..10 { assert!(control.sample(tick as f64 * 0.01, Some(0.0)).0); }
        let (fast, slope) = control.sample(0.1, Some(9.0));
        assert!(fast);
        assert!(slope.unwrap() > 10.0);
    }
}
