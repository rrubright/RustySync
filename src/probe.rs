use std::fs;

use crate::nvme;
use crate::types::{DiskStats, DriveId, DriveSample};

pub fn probe(device: &DriveId) -> Result<DriveSample, Box<dyn std::error::Error>> {
    let disk_stats = read_disk_stats(&device.name)?;

    let temperature_c = match nvme::temperature_millicelsius(&device.name) {
        Ok(milli_c) => Some(f64::from(milli_c) / 1000.0),
        Err(error) => {
            eprintln!("TEMP  {} unavailable: {}", device.name, error);
            None
        }
    };
    let (temperature_millicelsius, temperature_c) =
        match nvme::temperature_millicelsius(&device.name) {
            Ok(milli_c) => (Some(milli_c), Some(f64::from(milli_c) / 1000.0)),
            Err(error) => {
                eprintln!("TEMP  {} unavailable: {}", device.name, error);
                (None, None)
            }
        };

    Ok(DriveSample {
        id: device.clone(),
        temperature_millicelsius,
        temperature_c,
        disk_stats: Some(disk_stats),
        write_latency_ms: None,
        bytes_written: None,
    })
}

fn read_disk_stats(device: &str) -> Result<DiskStats, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string("/proc/diskstats")?;

    let line = contents
        .lines()
        .find(|line| line.split_whitespace().nth(2) == Some(device))
        .ok_or_else(|| format!("device not found in /proc/diskstats: {}", device))?;

    let fields = line
        .split_whitespace()
        .skip(3)
        .map(str::parse)
        .collect::<Result<Vec<u64>, _>>()?;

    Ok(DiskStats { fields })
}
