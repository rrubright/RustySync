//! Burst-assessment policy, currently not called by the runtime controller.
//! Burst lengths count samples with writes, not seconds or elapsed ticks.

use crate::algs::RunningAverage;

/// Shorter bursts are dominated by rsync startup overhead and
/// do not provide a meaningful assessment.
const MIN_MEANINGFUL_BURST: u32 = 5;

/// Drives sustaining writes for this long are considered
/// immediately suitable for normal operation.
const ASSESSMENT_COMPLETE_BURST: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    Complete,
    CantContinue,
}

pub struct Assessment {
    // Best Burst So Far: longest sustained burst observed.
    bbsf: Option<u32>,
}

impl Assessment {
    pub fn new() -> Self {
        Self { bbsf: None }
    }

    pub fn record_burst(&mut self, burst: u32) -> Verdict {
        if burst < MIN_MEANINGFUL_BURST {
            return Verdict::CantContinue;
        }

        if burst >= ASSESSMENT_COMPLETE_BURST {
            self.bbsf = Some(burst);
            return Verdict::Complete;
        }

        match self.bbsf {
            None => {
                self.bbsf = Some(burst);
                Verdict::Continue
            }
            Some(bbsf) if burst > bbsf => {
                self.bbsf = Some(burst);
                Verdict::Continue
            }
            Some(_) => Verdict::Complete,
        }
    }
}

impl Default for Assessment {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct BurstTracker {
    active: bool,
    length: u32,
    latency_average: RunningAverage,
    floor_latency_ms: Option<f64>,
    baseline_samples: Vec<f64>,
    consecutive_rises: u32,
}

const BASELINE_SAMPLES: usize = 4;
const MIN_LATENCY_RISE_MS: f64 = 4.0;
const SIGMA_MULTIPLIER: f64 = 3.0;
const BASELINE_RATIO: f64 = 1.5;
const REQUIRED_CONSECUTIVE_RISES: u32 = 2;

impl BurstTracker {
    pub fn new(smoothing_frames: usize) -> Self {
        Self {
            active: false,
            length: 0,
            latency_average: RunningAverage::new(smoothing_frames),
            floor_latency_ms: None,
            baseline_samples: Vec::new(),
            consecutive_rises: 0,
        }
    }

    // Idle or missing-write samples do not end an active burst. Only a
    // persistent latency rise emits its length; the triggering sample is excluded.
    pub fn record_sample(
        &mut self,
        bytes_written: Option<u64>,
        write_latency_ms: Option<f64>,
    ) -> Option<u32> {
        let writing = bytes_written.is_some_and(|bytes| bytes > 0);

        let smoothed_latency_ms =
            write_latency_ms.map(|latency| self.latency_average.push(latency));

        if !self.active {
            if writing {
                self.active = true;
                self.length = 1;
            }

            return None;
        }

        if let Some(current) = smoothed_latency_ms {
            if self.baseline_samples.len() < BASELINE_SAMPLES {
                self.baseline_samples.push(current);

                self.floor_latency_ms = Some(
                    self.floor_latency_ms
                        .map_or(current, |floor| floor.min(current)),
                );
            } else {
                let floor = self.floor_latency_ms.unwrap_or(current);

                let mean =
                    self.baseline_samples.iter().sum::<f64>() / self.baseline_samples.len() as f64;

                let variance = self
                    .baseline_samples
                    .iter()
                    .map(|sample| {
                        let delta = sample - mean;
                        delta * delta
                    })
                    .sum::<f64>()
                    / self.baseline_samples.len() as f64;

                let sigma = variance.sqrt();
                let rise_threshold = MIN_LATENCY_RISE_MS.max(SIGMA_MULTIPLIER * sigma);

                // Either an absolute/noise-adjusted rise OR a relative rise
                // qualifies. The ratio test can fire below MIN_LATENCY_RISE_MS.
                let triggered =
                    current - floor >= rise_threshold || current >= mean * BASELINE_RATIO;

                if triggered {
                    self.consecutive_rises += 1;
                } else {
                    self.consecutive_rises = 0;
                    self.floor_latency_ms = Some(floor.min(current));
                }

                if self.consecutive_rises >= REQUIRED_CONSECUTIVE_RISES {
                    let completed = self.length;

                    self.active = false;
                    self.length = 0;
                    self.floor_latency_ms = None;
                    self.baseline_samples.clear();
                    self.consecutive_rises = 0;
                    self.latency_average.clear();

                    return Some(completed);
                }
            }
        }

        if writing {
            self.length += 1;
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_burst_continues_assessment() {
        let mut assessment = Assessment::new();

        let verdict = assessment.record_burst(100);

        assert_eq!(verdict, Verdict::Continue);
    }

    #[test]
    fn improved_burst_continues_assessment() {
        let mut assessment = Assessment::new();

        assessment.record_burst(100);
        let verdict = assessment.record_burst(200);

        assert_eq!(verdict, Verdict::Continue);
    }

    #[test]
    fn equal_burst_completes_assessment() {
        let mut assessment = Assessment::new();

        assessment.record_burst(100);
        let verdict = assessment.record_burst(100);

        assert_eq!(verdict, Verdict::Complete);
    }

    #[test]
    fn shorter_burst_completes_assessment() {
        let mut assessment = Assessment::new();

        assessment.record_burst(200);
        let verdict = assessment.record_burst(100);

        assert_eq!(verdict, Verdict::Complete);
    }

    #[test]
    fn bbsf_tracks_best_burst_not_previous_burst() {
        let mut assessment = Assessment::new();

        assessment.record_burst(100);
        assessment.record_burst(250);
        let verdict = assessment.record_burst(200);

        assert_eq!(verdict, Verdict::Complete);
    }

    #[test]
    fn very_short_first_burst_cannot_continue() {
        let mut assessment = Assessment::new();

        let verdict = assessment.record_burst(MIN_MEANINGFUL_BURST - 1);

        assert_eq!(verdict, Verdict::CantContinue);
    }

    #[test]
    fn sufficient_first_burst_continues_assessment() {
        let mut assessment = Assessment::new();

        let verdict = assessment.record_burst(MIN_MEANINGFUL_BURST);

        assert_eq!(verdict, Verdict::Continue);
    }

    #[test]
    fn long_first_burst_completes_assessment() {
        let mut assessment = Assessment::new();

        let verdict = assessment.record_burst(ASSESSMENT_COMPLETE_BURST);

        assert_eq!(verdict, Verdict::Complete);
    }

    #[test]
    fn burst_starts_on_write_and_ends_after_persistent_latency_rise() {
        let mut burst = BurstTracker::new(1);

        assert_eq!(burst.record_sample(Some(100), Some(1.0)), None);
        assert_eq!(burst.record_sample(Some(200), Some(1.0)), None);
        assert_eq!(burst.record_sample(Some(300), Some(1.0)), None);
        assert_eq!(burst.record_sample(Some(400), Some(1.0)), None);

        assert_eq!(burst.record_sample(Some(500), Some(1.0)), None);

        assert_eq!(burst.record_sample(Some(600), Some(6.0)), None);
        assert_eq!(burst.record_sample(Some(700), Some(6.0)), Some(6));
    }
}
