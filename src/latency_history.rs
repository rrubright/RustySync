//! Characterization only: independent least-squares latency trends per drive.
use std::collections::{HashMap, VecDeque};
use crate::types::DriveSample;

#[derive(Default)]
pub struct LatencyHistory {
    samples: HashMap<String, VecDeque<(f64, f64)>>,
}

impl LatencyHistory {
    pub fn sample(&mut self, seconds: f64, drives: &[DriveSample]) -> HashMap<String, f64> {
        let mut slopes = HashMap::new();
        if !seconds.is_finite() {
            return slopes;
        }
        for history in self.samples.values_mut() {
            if history.back().is_some_and(|&(t, _)| t > seconds) {
                history.clear();
            }
            while history.front().is_some_and(|&(t, _)| seconds - t > 30.0) {
                history.pop_front();
            }
        }
        for drive in drives {
            let history = self.samples.entry(drive.id.name.clone()).or_default();
            if let Some(latency) = drive.write_latency_ms.filter(|v| v.is_finite() && *v >= 0.0) {
                // A repeated timestamp is one observation, not extra regression weight.
                if history.back().is_some_and(|&(t, _)| t == seconds) {
                    history.pop_back();
                }
                history.push_back((seconds, latency));
            }
            if let Some(slope) = regression(history) {
                slopes.insert(drive.id.name.clone(), slope);
            }
        }
        slopes
    }
}

fn regression(samples: &VecDeque<(f64, f64)>) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let origin = samples.front()?.0;
    let n = samples.len() as f64;
    let mean_t = samples.iter().map(|&(t, _)| t - origin).sum::<f64>() / n;
    let mean_y = samples.iter().map(|&(_, y)| y).sum::<f64>() / n;
    let mut covariance = 0.0;
    let mut variance = 0.0;
    for &(t, y) in samples {
        let dt = (t - origin) - mean_t;
        covariance += dt * (y - mean_y);
        variance += dt * dt;
    }
    let slope = covariance / variance;
    (variance > 0.0 && slope.is_finite()).then_some(slope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DriveId;
    fn drive(name: &str, latency: Option<f64>) -> DriveSample {
        DriveSample { id: DriveId { name: name.into() }, temperature_millicelsius: None,
            temperature_c: None, disk_stats: None, write_latency_ms: latency, bytes_written: None }
    }
    #[test]
    fn independent_slopes_with_irregular_times_and_reordered_drives() {
        let mut history = LatencyHistory::default();
        assert!(history.sample(0.0, &[drive("a", Some(10.0))]).is_empty());
        for t in [1.0, 4.0, 9.0, 30.0] {
            let slopes = history.sample(t, &[drive("b", Some(100.0 - t)), drive("a", Some(10.0 + 2.0*t))]);
            assert!((slopes["a"] - 2.0).abs() < 1e-10);
            if t > 1.0 { assert!((slopes["b"] + 1.0).abs() < 1e-10); }
        }
    }
    #[test]
    fn invalid_samples_are_skipped_and_old_samples_expire() {
        let mut history = LatencyHistory::default();
        history.sample(0.0, &[drive("a", Some(0.0))]);
        history.sample(10.0, &[drive("a", Some(20.0))]);
        for (t, value) in [(11.0, None), (12.0, Some(f64::NAN)), (13.0, Some(-1.0)), (14.0, Some(f64::INFINITY))] {
            assert_eq!(history.sample(t, &[drive("a", value)])["a"], 2.0);
        }
        assert!(history.sample(41.0, &[drive("a", None)]).is_empty());
        assert!(history.sample(42.0, &[drive("a", Some(5.0))]).is_empty());
        assert_eq!(history.sample(43.0, &[drive("a", Some(5.0))])["a"], 0.0);
    }
    #[test]
    fn regression_uses_interior_samples_and_exact_window_boundary() {
        let mut history = LatencyHistory::default();
        history.sample(0.0, &[drive("a", Some(0.0))]);
        history.sample(10.0, &[drive("a", Some(20.0))]);
        let slopes = history.sample(30.0, &[drive("a", Some(0.0))]);
        assert!((slopes["a"] + 1.0/7.0).abs() < 1e-10);
        assert!(history.sample(61.0, &[drive("a", Some(0.0))]).is_empty());
    }
}
