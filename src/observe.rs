//! Collects one snapshot of raw system and storage facts.
//!
//! Observation does not interpret measurements or make decisions.
//! Delta calculations belong in `algs`; policy belongs in state and assessment.

use std::time::SystemTime;

use crate::types::{Capabilities, Configuration, MissingData, Observation};

pub fn observe(_config: &Configuration, capabilities: &Capabilities) -> Observation {
    let mut observation = Observation::new();
    observation.timestamp = SystemTime::now();

    for drive in &capabilities.drives {
        match crate::probe::probe(&drive.id) {
            Ok(sample) => {
                if drive.reports_temperature && sample.temperature_c.is_none() {
                    observation.missing(MissingData::NvmeTemperature(drive.id.clone()));
                }

                observation.add_drive(sample);
            }
            Err(error) => {
                eprintln!("PROBE {} failed: {}", drive.id.name, error);
                observation.missing(MissingData::WriteCounters(drive.id.clone()));
            }
        }
    }

    if let Some(bytes) = crate::launch::progress_bytes() {
        observation.set_rsync_progress(bytes);
    } else {
        observation.missing(MissingData::RsyncProgress);
    }

    observation
}
