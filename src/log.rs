use std::fs::OpenOptions;
use std::io::Write;

use crate::types::Observation;

pub fn append_observation(
    path: &str,
    observation: &Observation,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;

    let timestamp = observation
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    for drive in &observation.drives {
        let Some(stats) = drive.disk_stats.as_ref() else {
            continue;
        };

        writeln!(
            file,
            "{},{},{},{},{},{},{},{}",
            timestamp,
            drive.id.name,
            drive.temperature_millicelsius.unwrap_or(0),
            stats.fields[4],
            stats.fields[6],
            stats.fields[7],
            observation.rsync_progress_bytes.unwrap_or(0),
            observation.rsync_velocity_mb_s.unwrap_or(0.0),
        )?;
    }

    Ok(())
}
