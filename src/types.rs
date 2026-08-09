use std::collections::VecDeque;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct DiskStats {
    // Formerly:
    // pub writes_completed: u64,
    // pub write_time_ms: u64,

    // Raw numeric fields from /proc/diskstats after the device name.
    // We intentionally record the complete statistics row and decide
    // later which fields are useful for LoadLevel's algorithms.
    pub fields: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct Configuration {
    pub sample_interval_ms: u64,
    pub calibration_samples: usize,
    pub pause_write_latency_ms: f64,
    pub max_write_latency_ms: f64,
    pub topology: Option<StorageTopology>,
    pub smoothing_frames: usize,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            sample_interval_ms: 100,
            calibration_samples: 10,
            pause_write_latency_ms: 2000.0,
            smoothing_frames: 5,
            max_write_latency_ms: 10000.0,
            topology: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub drives: Vec<DriveCapability>,
}

#[derive(Debug, Clone)]
pub struct DriveCapability {
    pub id: DriveId,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub reports_temperature: bool,
    pub reports_write_latency: bool,
    pub reports_write_counters: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriveId {
    pub name: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageLayout {
    Direct,
    MdRaid1,
    MdRaid10,
    MdRaid5or6,
    Lvm,
    LvmOverRaid,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanaryRole {
    Logical,
    Physical,
    // Compartment-level safety sentinel, such as CPU "Package id 0".
    // It protects the system from shared cooling failure and does not
    // participate in storage burst characterization.
    Thermal,
}

#[derive(Debug, Clone)]
pub struct Canary {
    pub id: DriveId,
    pub role: CanaryRole,
}

#[derive(Debug, Clone)]
pub struct StorageTopology {
    pub destination_path: String,
    pub logical_device: DriveId,
    pub layout: StorageLayout,
    pub canaries: Vec<Canary>,
}
#[derive(Debug, Clone)]
pub struct Observation {
    pub timestamp: SystemTime,
    pub drives: Vec<DriveSample>,
    pub rsync_progress_bytes: Option<u64>,
    pub rsync_velocity_mb_s: Option<f64>,
    pub missing_flags: Vec<MissingData>,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            timestamp: SystemTime::UNIX_EPOCH,
            drives: Vec::new(),
            rsync_progress_bytes: None,
            rsync_velocity_mb_s: None,
            missing_flags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SampleLog {
    pub samples: VecDeque<Observation>,
    pub max_samples: usize,
}

impl SampleLog {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    pub fn push(&mut self, observation: Observation) {
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }

        self.samples.push_back(observation);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}
#[derive(Debug, Clone)]
pub struct DriveSample {
    pub id: DriveId,
    pub temperature_millicelsius: Option<i32>,
    pub temperature_c: Option<f64>,
    pub disk_stats: Option<DiskStats>,
    pub write_latency_ms: Option<f64>,
    pub bytes_written: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingData {
    NvmeTemperature(DriveId),
    WriteLatency(DriveId),
    WriteCounters(DriveId),
    RsyncProgress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Ignore,
    Warning,
    Pause,
    Abort,
}

#[derive(Debug, Clone)]
pub enum ThrottleAction {
    Hold,
    Increase(u16),
    Decrease(u16),
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub severity: Severity,
    pub message: Option<String>,
    pub action: ThrottleAction,
}
#[derive(Debug, Clone)]
pub struct Sample {
    pub nvme_temperature_c: Option<i32>,
    pub write_latency_ms: Option<f64>,
    pub bytes_copied: Option<u64>,
}

impl Observation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_drive(&mut self, drive: DriveSample) {
        self.drives.push(drive);
    }

    pub fn set_rsync_progress(&mut self, bytes: u64) {
        self.rsync_progress_bytes = Some(bytes);
    }

    pub fn missing(&mut self, flag: MissingData) {
        self.missing_flags.push(flag);
    }
}
