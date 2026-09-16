use crate::types::{Decision, DiskStats, MissingData, Observation, Severity, ThrottleAction};
use std::collections::VecDeque;

#[derive(Debug)]
pub struct RunningAverage {
    window: VecDeque<f64>,
    max_samples: usize,
    sum: f64,
}

impl RunningAverage {
    pub fn new(max_samples: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(max_samples),
            max_samples,
            sum: 0.0,
        }
    }

    pub fn push(&mut self, value: f64) -> f64 {
        if self.window.len() == self.max_samples {
            if let Some(oldest) = self.window.pop_front() {
                self.sum -= oldest;
            }
        }

        self.window.push_back(value);
        self.sum += value;

        self.sum / self.window.len() as f64
    }
    pub fn clear(&mut self) {
        self.window.clear();
        self.sum = 0.0;
    }
}
/// Compute average write latency in milliseconds per completed write.

pub fn write_latency_ms(old: &DiskStats, new: &DiskStats) -> Option<f64> {
    // DiskStats::fields starts after major, minor, and device name.
    // Therefore:
    // fields[4] = writes completed  (zero-based column 7)
    // fields[7] = write time in ms (zero-based column 10)
    let old_writes = *old.fields.get(4)?;
    let old_time_ms = *old.fields.get(7)?;
    let new_writes = *new.fields.get(4)?;
    let new_time_ms = *new.fields.get(7)?;

    /*
    println!(
        "DEBUG old: writes={} time={}  new: writes={} time={}",
        old_writes, old_time_ms, new_writes, new_time_ms
    );
    */
    // A reset/decrease or an interval without completed writes has no usable
    // latency delta; None is not evidence of zero latency.
    let writes_delta = new_writes.checked_sub(old_writes)?;
    let time_delta = new_time_ms.checked_sub(old_time_ms)?;

    if writes_delta == 0 {
        return None;
    }

    /*
    println!(
        "DEBUG writes_delta={} time_delta={}ms",
        writes_delta, time_delta
    );
    */

    Some(time_delta as f64 / writes_delta as f64)
}

pub fn drive_latency_ms(
    old: &crate::types::DriveSample,
    new: &crate::types::DriveSample,
) -> Option<f64> {
    write_latency_ms(old.disk_stats.as_ref()?, new.disk_stats.as_ref()?)
}
pub fn drive_bytes_written(
    old: &crate::types::DriveSample,
    new: &crate::types::DriveSample,
) -> Option<u64> {
    // DiskStats::fields starts after major, minor, and device name.
    // fields[6] = sectors written (zero-based column 9).
    let old_sectors = *old.disk_stats.as_ref()?.fields.get(6)?;
    let new_sectors = *new.disk_stats.as_ref()?.fields.get(6)?;

    let sectors_written = new_sectors.checked_sub(old_sectors)?;

    // Linux diskstats sectors are traditionally 512 bytes.
    sectors_written.checked_mul(512)
}
pub fn evaluate(old: &Observation, new: &Observation) -> Decision {
    let mut severity = Severity::Ignore;
    let mut messages = Vec::new();

    for (old_drive, new_drive) in old.drives.iter().zip(new.drives.iter()) {
        let latency = drive_latency_ms(old_drive, new_drive);

        match latency {
            Some(ms) => println!("{} latency = {:.3} ms/write", new_drive.id.name, ms),
            None => println!("{} latency = idle/no writes", new_drive.id.name),
        }
    }
    for flag in &new.missing_flags {
        let (level, text) = classify(flag);

        if rank(&level) > rank(&severity) {
            severity = level.clone();
        }

        messages.push(text);
    }

    Decision {
        severity,
        message: if messages.is_empty() {
            None
        } else {
            Some(messages.join("; "))
        },
        action: ThrottleAction::Hold,
    }
}

fn classify(flag: &MissingData) -> (Severity, String) {
    match flag {
        MissingData::RsyncProgress => (Severity::Warning, "Missing rsync progress".into()),

        MissingData::NvmeTemperature(id) => (
            Severity::Warning,
            format!("Missing temperature for {}", id.name),
        ),

        MissingData::WriteCounters(id) => (
            Severity::Pause,
            format!("Missing write counters for {}", id.name),
        ),

        MissingData::WriteLatency(id) => (
            Severity::Abort,
            format!("Missing write latency for {}", id.name),
        ),
    }
}

fn rank(level: &Severity) -> u8 {
    match level {
        Severity::Ignore => 0,
        Severity::Warning => 1,
        Severity::Pause => 2,
        Severity::Abort => 3,
    }
}
pub fn rsync_velocity_mb_s(old: &Observation, new: &Observation) -> Option<f64> {
    let old_bytes = old.rsync_progress_bytes?;
    let new_bytes = new.rsync_progress_bytes?;

    let elapsed = new
        .timestamp
        .duration_since(old.timestamp)
        .ok()?
        .as_secs_f64();

    if elapsed <= 0.0 || new_bytes < old_bytes {
        return None;
    }

    Some((new_bytes - old_bytes) as f64 / elapsed / 1_000_000.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DiskStats, DriveId, DriveSample};

    fn sample_with_sectors(name: &str, sectors_written: u64) -> DriveSample {
        let mut fields = vec![0; 8];
        fields[6] = sectors_written;

        DriveSample {
            id: DriveId {
                name: name.to_string(),
            },

            temperature_millicelsius: None,
            temperature_c: None,
            disk_stats: Some(DiskStats { fields }),
            write_latency_ms: None,
            bytes_written: None,
        }
    }

    #[test]
    fn calculates_bytes_written_from_sector_delta() {
        let old = sample_with_sectors("nvme0n1", 100);
        let new = sample_with_sectors("nvme0n1", 108);

        assert_eq!(drive_bytes_written(&old, &new), Some(4096));
    }

    fn sample_with_write_stats(
        name: &str,
        writes_completed: u64,
        write_time_ms: u64,
    ) -> DriveSample {
        let mut fields = vec![0; 8];
        fields[4] = writes_completed;
        fields[7] = write_time_ms;

        DriveSample {
            id: DriveId {
                name: name.to_string(),
            },
            temperature_millicelsius: None,
            temperature_c: None,
            disk_stats: Some(DiskStats { fields }),
            write_latency_ms: None,
            bytes_written: None,
        }
    }

    #[test]
    fn calculates_write_latency_from_deltas() {
        let old = sample_with_write_stats("nvme0n1", 100, 500);
        let new = sample_with_write_stats("nvme0n1", 104, 520);

        assert_eq!(drive_latency_ms(&old, &new), Some(5.0));
    }
}

/// Time-windowed progress rate; repeated counters are real zero progress,
/// while missing counters invalidate the window rather than inventing a rate.
pub struct ProgressRate {
    samples: VecDeque<(f64, u64)>,
}
impl ProgressRate {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
        }
    }
    pub fn sample(&mut self, seconds: f64, bytes: Option<u64>) -> Option<f64> {
        let Some(bytes) = bytes else {
            self.samples.clear();
            return None;
        };
        if self
            .samples
            .back()
            .is_some_and(|&(t, b)| seconds <= t || bytes < b)
        {
            self.samples.clear();
        }
        self.samples.push_back((seconds, bytes));
        while self.samples.len() > 2 && self.samples[1].0 <= seconds - 10.0 {
            self.samples.pop_front();
        }
        let &(start, first_bytes) = self.samples.front()?;
        let elapsed = seconds - start;
        (elapsed > 0.0).then(|| (bytes - first_bytes) as f64 / elapsed / 1_000_000.0)
    }
}
#[cfg(test)]
mod progress_rate_tests {
    use super::*;
    #[test]
    fn averages_batched_progress_and_ages_out_old_data() {
        let mut rate = ProgressRate::new();
        assert_eq!(rate.sample(0.0, Some(0)), None);
        for t in 1..=10 {
            rate.sample(t as f64, Some(if t < 10 { 0 } else { 100_000_000 }));
        }
        assert_eq!(rate.sample(11.0, Some(100_000_000)), Some(10.0));
        assert_eq!(rate.sample(21.0, Some(100_000_000)), Some(0.0));
    }
    #[test]
    fn missing_and_reset_counters_require_a_new_baseline() {
        let mut rate = ProgressRate::new();
        rate.sample(0.0, Some(100));
        assert_eq!(rate.sample(1.0, Some(0)), None);
        assert_eq!(rate.sample(2.0, None), None);
        assert_eq!(rate.sample(3.0, Some(100)), None);
    }
}
