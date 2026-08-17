use std::fs;

use crate::nvme;
use crate::types::{DiskStats, DriveId, DriveSample};

pub fn probe(device: &DriveId) -> Result<DriveSample, Box<dyn std::error::Error>> {
    let disk_stats = read_disk_stats(&device.name)?;
    let (temperature_millicelsius, temperature_c) =
        temperature_pair(nvme::temperature_millicelsius(&device.name), &device.name);
    Ok(DriveSample {
        id: device.clone(),
        temperature_millicelsius,
        temperature_c,
        disk_stats: Some(disk_stats),
        write_latency_ms: None,
        bytes_written: None,
    })
}
fn temperature_pair(result: Result<i32, String>, device: &str) -> (Option<i32>, Option<f64>) {
    match result {
        Ok(milli_c) => (Some(milli_c), Some(f64::from(milli_c) / 1000.0)),
        Err(error) => {
            eprintln!("TEMP  {} unavailable: {}", device, error);
            (None, None)
        }
    }
}
fn read_disk_stats(device: &str) -> Result<DiskStats, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string("/proc/diskstats")?;
    parse_disk_stats(&contents, device)
}

fn parse_disk_stats(contents: &str, device: &str) -> Result<DiskStats, Box<dyn std::error::Error>> {
    let line = contents
        .lines()
        .find(|line| line.split_whitespace().nth(2) == Some(device))
        .ok_or_else(|| format!("device not found in /proc/diskstats: {}", device))?;

    let fields = line
        .split_whitespace()
        .skip(3)
        .map(str::parse)
        .collect::<Result<Vec<u64>, _>>()?;
    if fields.len() <= 7 {
        return Err(format!(
            "incomplete /proc/diskstats entry for {}: {} fields",
            device,
            fields.len()
        )
        .into());
    }
    Ok(DiskStats { fields })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_write_counter_positions() {
        let contents = "259 0 nvme0n1 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26\n";

        let stats = parse_disk_stats(contents, "nvme0n1").unwrap();

        assert_eq!(stats.fields[4], 14); // writes completed
        assert_eq!(stats.fields[6], 16); // sectors written
        assert_eq!(stats.fields[7], 17); // write time ms
    }
    #[test]
    fn rejects_incomplete_disk_stats() {
        let contents = "259 0 nvme0n1 1 2 3 4 5 6 7\n";

        let result = parse_disk_stats(contents, "nvme0n1");

        assert!(result.is_err());
    }
    #[test]
    fn parses_matching_device_stats() {
        let contents = "259 0 nvme0n1 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17\n";

        let stats = parse_disk_stats(contents, "nvme0n1").unwrap();

        assert_eq!(stats.fields[0], 1);
        assert_eq!(stats.fields[4], 5);
        assert_eq!(stats.fields[7], 8);
    }

    #[test]
    fn missing_device_returns_error() {
        let contents = "259 0 nvme0n1 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17\n";

        let result = parse_disk_stats(contents, "nvme9n9");

        assert!(result.is_err());
    }
}
