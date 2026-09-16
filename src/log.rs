use std::fs::OpenOptions;
use std::io::Write;

use crate::types::Observation;

const CSV_HEADER: &str = "timestamp,drive,temperature_millicelsius,writes,sectors_written,write_time_ms,weighted_io_time_ms,rsync_progress_bytes,rsync_velocity_mb_s,poke_state,bwlimit_kb_s,write_latency_ms,canary_latency_slope_ms_per_s";

// Keep older logs intact: an incompatible header selects a separate v2 file.
// Missing optional measurements are blank, while measured zeros remain zero.
pub fn append_observation(
    path: &str,
    observation: &Observation,
    fast: bool,
    kb_per_sec: u64,
    slope: Option<f64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut path = std::path::PathBuf::from(path);
    if !compatible_header(&path)? {
        path.set_extension("v2.csv");
        if !compatible_header(&path)? {
            return Err(format!("incompatible CSV header: {}", path.display()).into());
        }
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if file.metadata()?.len() == 0 {
        writeln!(file, "{}", CSV_HEADER)?;
    }
    write_rows(&mut file, observation, fast, kb_per_sec, slope)?;
    Ok(())
}

fn compatible_header(path: &std::path::Path) -> std::io::Result<bool> {
    use std::io::BufRead;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    let mut header = String::new();
    std::io::BufReader::new(file).read_line(&mut header)?;
    Ok(header.is_empty() || header.trim_end() == CSV_HEADER)
}

fn optional(value: Option<impl std::fmt::Display>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn write_rows(
    file: &mut impl Write,
    observation: &Observation,
    fast: bool,
    kb_per_sec: u64,
    slope: Option<f64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = observation
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    // Retain a state row even when the destination probe produced no sample.
    let drives: Vec<_> = if observation.drives.is_empty() {
        vec![None]
    } else {
        observation.drives.iter().map(Some).collect()
    };
    for drive in drives {
        let stats = drive.and_then(|d| d.disk_stats.as_ref());
        let counter = |index: usize| stats.and_then(|s| s.fields.get(index).copied());
        writeln!(
            file,
            "{:.3},{},{},{},{},{},{},{},{},{},{},{},{}",
            timestamp,
            drive.map(|d| d.id.name.as_str()).unwrap_or(""),
            optional(drive.and_then(|d| d.temperature_millicelsius)),
            optional(counter(4)),
            optional(counter(6)),
            optional(counter(7)),
            optional(counter(10)),
            optional(observation.rsync_progress_bytes),
            optional(observation.rsync_velocity_mb_s),
            if fast { "FAST_POKE" } else { "SLOW_POKE" },
            kb_per_sec,
            optional(drive.and_then(|d| d.write_latency_ms)),
            optional(slope)
        )?;
    }
    Ok(())
}

/// Console companion to the observation CSV. Missing measurements remain
/// explicit; they do not imply an idle or recovered destination.
pub fn report_poke(
    event: &str,
    drive: &str,
    fast: bool,
    kb_per_sec: u64,
    slope_ms_per_sec: Option<f64>,
    latency_ms: Option<f64>,
    velocity_mb_s: Option<f64>,
) {
    if event == "POKE INITIAL" {
        println!("Live status every 5 s; speed = rsync progress averaged over up to 10 s; -- = unavailable");
        println!(
            "{:<7} {:<12} {:<5} {:>12} {:>12} {:>12} {:>12}",
            "EVENT", "CANARY", "STATE", "LIMIT KiB/s", "LATENCY ms", "SPEED MiB/s", "SLOPE ms/s"
        );
    }
    let number = |v: Option<f64>| v.map(|v| format!("{v:.2}")).unwrap_or_else(|| "--".into());
    let row = format!(
        "{:<7} {:<12} {:<5} {:>12} {:>12} {:>12} {:>12}",
        match event {
            "POKE INITIAL" => "START",
            "POKE CHANGE" => "CHANGE",
            "CANARY" => "CANARY",
            _ => "STATUS",
        },
        drive,
        if fast { "FAST" } else { "SLOW" },
        kb_per_sec,
        number(latency_ms),
        number(velocity_mb_s.map(|v| v * 1_000_000.0 / 1_048_576.0)),
        number(slope_ms_per_sec)
    );
    println!("{row}");
    if event != "STATUS" {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        eprintln!("[{timestamp}] {row}");
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_telemetry_preserves_state_and_csv_shape() {
        let mut output = Vec::new();
        let mut observation = Observation::new();
        observation.timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1234);
        write_rows(&mut output, &observation, false, 1000, None).unwrap();
        let text = String::from_utf8(output).unwrap();
        let fields: Vec<_> = text.trim_end().split(',').collect();
        assert_eq!(fields.len(), CSV_HEADER.split(',').count());
        assert_eq!(fields[0], "1.234");
        assert_eq!(&fields[9..], &["SLOW_POKE", "1000", "", ""]);
    }

    #[test]
    fn measured_zero_slope_is_not_missing() {
        let mut output = Vec::new();
        write_rows(&mut output, &Observation::new(), true, 40000, Some(0.0)).unwrap();
        assert!(String::from_utf8(output)
            .unwrap()
            .ends_with(",FAST_POKE,40000,,0\n"));
    }
}
