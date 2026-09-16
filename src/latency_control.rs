use crate::algs::RunningAverage;

const SMOOTHING_SAMPLES: usize = 10;
// Low validation threshold: exercise SLOW before a severe latency spike.
pub const SLOW_SLOPE_MS_PER_SEC: f64 = 10.0;

/// Rusty's latched bandwidth decision. Time is monotonic elapsed seconds.
pub struct LatencyControl {
    fast: bool,
    average: RunningAverage,
    samples: usize,
    previous: Option<(f64, f64)>,
}

impl LatencyControl {
    pub fn new() -> Self {
        Self {
            fast: true,
            average: RunningAverage::new(SMOOTHING_SAMPLES),
            samples: 0,
            previous: None,
        }
    }

    fn reset_trend(&mut self) {
        self.average.clear();
        self.samples = 0;
        self.previous = None;
    }

    pub fn sample(&mut self, seconds: f64, latency: Option<f64>) -> (bool, Option<f64>) {
        let Some(latency) = latency.filter(|v| v.is_finite() && *v >= 0.0) else {
            // Do not bridge an unknown interval with an apparently valid slope.
            // Discard trend history, but preserve the latched bandwidth state.
            self.reset_trend();
            return (self.fast, None);
        };
        if !self.fast && latency < 1.0 {
            self.fast = true;
            self.reset_trend();
        }
        let smoothed = self.average.push(latency);
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
        if self.fast && slope.is_some_and(|v| v >= SLOW_SLOPE_MS_PER_SEC) {
            self.fast = false;
        }
        (self.fast, slope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(rate: f64) -> LatencyControl {
        let mut rusty = LatencyControl::new();
        for tick in 0..20 {
            let t = tick as f64 * 0.125;
            rusty.sample(t, Some(10.0 + rate * t));
        }
        rusty
    }

    #[test]
    fn validation_threshold_is_ten_ms_per_second() {
        assert!(ramp(9.0).fast);
        assert!(!ramp(10.0).fast);
        assert!(!ramp(12.0).fast);
    }

    #[test]
    fn missing_samples_hold_congestion_and_recovery_is_strict() {
        let mut rusty = ramp(1200.0);
        assert_eq!(rusty.sample(3.0, None), (false, None));
        assert_eq!(rusty.sample(3.1, Some(1.0)), (false, None));
        assert_eq!(rusty.sample(3.2, Some(0.9)), (true, None));
        assert_eq!(rusty.sample(3.3, None), (true, None));
    }

    #[test]
    fn noise_and_missing_data_do_not_invent_a_rise() {
        let mut rusty = LatencyControl::new();
        for tick in 0..30 {
            let latency = if tick % 2 == 0 { 10.0 } else { 12.0 };
            assert!(rusty.sample(tick as f64 * 0.1, Some(latency)).0);
        }
        assert_eq!(rusty.sample(3.0, None), (true, None));
        assert_eq!(rusty.sample(3.1, Some(5000.0)), (true, None));
    }
}
