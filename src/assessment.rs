use crate::algs::RunningAverage;
// Shorter bursts are dominated by rsync startup overhead and
/// do not provide a meaningful assessment.
const MIN_MEANINGFUL_BURST: u32 = 5;

/// Drives sustaining writes for this long are considered
/// immediately suitable for normal operation.
const ASSESSMENT_COMPLETE_BURST: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Assessment,
    Normal,
    Break,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Burst,
    Rest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    Complete,
    CantContinue,
}

pub struct Assessment {
    phase: Phase,

    // Best Burst So Far: longest sustained burst observed.
    bbsf: Option<u32>,
}

impl Assessment {
    pub fn new() -> Self {
        Self {
            phase: Phase::Burst,
            bbsf: None,
        }
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
    previous_latency_ms: Option<f64>,
    latency_average: RunningAverage,
}

impl BurstTracker {
    pub fn new(smoothing_frames: usize) -> Self {
        Self {
            active: false,
            length: 0,
            previous_latency_ms: None,
            latency_average: RunningAverage::new(smoothing_frames),
        }
    }

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
                self.previous_latency_ms = smoothed_latency_ms;
            }

            return None;
        }

        if let (Some(previous), Some(current)) = (self.previous_latency_ms, smoothed_latency_ms) {
            if current > previous {
                let completed = self.length;

                self.active = false;
                self.length = 0;
                self.previous_latency_ms = None;

                return Some(completed);
            }
        }

        if writing {
            self.length += 1;
        }

        if smoothed_latency_ms.is_some() {
            self.previous_latency_ms = smoothed_latency_ms;
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
    fn burst_starts_on_write_and_ends_when_latency_rises() {
        let mut burst = BurstTracker::new();

        assert_eq!(burst.record_sample(Some(100), Some(1.0)), None);
        assert_eq!(burst.record_sample(Some(200), Some(1.0)), None);
        assert_eq!(burst.record_sample(Some(300), Some(1.0)), None);
        assert_eq!(burst.record_sample(Some(400), Some(2.0)), Some(3));
    }
}
