mod algs;
mod assessment;
mod init;
mod launch;
mod log;
mod nvme;
mod observe;
mod phase;
mod probe;
mod state;
mod types;

use crate::phase::Phase;
use crate::state::State;
use crate::types::{Capabilities, Configuration, DriveCapability, DriveId, Observation, SampleLog};
use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::Duration;
#[derive(Debug, Clone, Copy)]
enum RecoveryState {
    Normal,
    Recovery { deadline: std::time::Instant },
    Probing { deadline: std::time::Instant },
} // Main control loop.
  //
  // Startup:
  //   - Initialize configuration and capabilities.
  //   - Collect calibration observations.
  //   - Establish baseline measurements.
  //
  // Runtime loop:
  //   - Acquire a new Observation.
  //   - Compute derived metrics (latency, transfer velocity, etc.).
  //   - Evaluate system state.
  //   - Apply decisions to rsync.
  //   - Log observations and decisions.
  //
  // Observe reports facts.
  // Algs derives meaning.
  // State decides.
  // Rsync executes.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("LoadLevel Rust {}", init::LOADLEVEL_VERSION);
    let mut nvme_names: Vec<String> = fs::read_dir("/sys/class/block")?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("nvme") && !name.contains('p'))
        .collect();

    nvme_names.sort();

    let drives = nvme_names
        .into_iter()
        .map(|name| DriveCapability {
            id: DriveId { name },
            model: None,
            serial: None,
            reports_temperature: true,
            reports_write_latency: false,
            reports_write_counters: false,
        })
        .collect();
    let mut state = State {
        phase: Phase::Startup,
        config: Configuration::default(),
        capabilities: Capabilities { drives },
        calibrated: false,
        last_decision: None,
    };

    let mut recovery = RecoveryState::Normal;

    let mut sample_log = SampleLog::new(state.config.calibration_samples);
    for _ in 0..128 {
        sample_log.push(Observation::new());
    }

    println!("After 128 pushes: {}", sample_log.len());

    sample_log.push(Observation::new());

    println!("After 129th push: {}", sample_log.len());

    init::init()?;

    println!("PASS  init");

    state.phase = Phase::Calibrating;

    let mut missing_counts: BTreeMap<String, usize> = BTreeMap::new();
    state.phase = Phase::Running;

    // let old = sample::sample(&state.config, &state.capabilities);
    // let new = sample::sample(&state.config, &state.capabilities);
    // let decision = algs::evaluate(&old, &new);
    // state.last_decision = Some(decision);

    // // println!("Write latency: {:?}", write_latency);
    /*
        // Legacy calibration/self-test loop.
        //
        // This loop was used during bring-up to verify observation,
        // temperature sensing, and missing-data detection before the
        // runtime rsync observation loop was implemented.
        //
        // The runtime loop below is now the primary observation engine.
        // This block is retained for regression testing and may later
        // become a dedicated self-test mode.

        for _ in 0..state.config.calibration_samples {
            let observation = observe::observe(&state.config, &state.capabilities);

            for drive in &observation.drives {
                if let (Some(mc), Some(tc)) = (
                    drive.temperature_millicelsius,
                    drive.temperature_c,
                ) {
                    println!(
                        "DRV {:7}  T={:6} mC ({:4.1} C)  writes={}  sectors={}  write_ms={}",
                        drive.id.name,
                        mc,
                        tc,
                        drive.disk_stats.as_ref().map_or(0, |d| d.fields[4]),
                        drive.disk_stats.as_ref().map_or(0, |d| d.fields[6]),
                        drive.disk_stats.as_ref().map_or(0, |d| d.fields[7]),
                    );
                }
                l
                for flag in &observation.missing_flags {
                    *missing_counts.entry(format!("{:?}", flag)).or_insert(0) += 1;
                }
            }
        }

        if !missing_counts.is_empty() {
            println!("Calibration summary:");

            for (flag, count) in &missing_counts {
                println!("WARN  {:<24} {:>2}/10", flag, count);
            }
        }

        println!("PASS  observe");
    */
    let mut old = observe::observe(&state.config, &state.capabilities);
    let mut burst_tracker = assessment::BurstTracker::new(state.config.smoothing_frames);
    let mut assessment = assessment::Assessment::new();

    let mut child = launch::launch(&[
        "-a",
        "--info=progress2",
        "--ignore-times",
        "/home/richard/Downloads/debian-links/",
        "/srv/canary/debian-links/",
    ])?;

    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                let mut new = observe::observe(&state.config, &state.capabilities);
                for (old_drive, new_drive) in old.drives.iter().zip(new.drives.iter_mut()) {
                    new_drive.write_latency_ms = algs::drive_latency_ms(old_drive, new_drive);
                    new_drive.bytes_written = algs::drive_bytes_written(old_drive, new_drive);
                }
                match recovery {
                    RecoveryState::Normal => {}
                    RecoveryState::Recovery { deadline } => {
                        if std::time::Instant::now() >= deadline {
                            recovery = RecoveryState::Probing {
                                deadline: std::time::Instant::now()
                                    + std::time::Duration::from_secs(5),
                            };

                            println!("Recovery timeout. Entering probing state.");
                        } else {
                            for drive in &new.drives {
                                if let Some(latency) = drive.write_latency_ms {
                                    if latency <= 100.0 {
                                        launch::cont(&child)?;

                                        // rsync child: SIGCONT sent.
                                        recovery = RecoveryState::Normal;

                                        println!("rsync child resumed.");

                                        break;
                                    }
                                }
                            }
                        }
                    }
                    RecoveryState::Probing { .. } => {
                        println!("Probing");
                    }
                }
                new.rsync_velocity_mb_s = algs::rsync_velocity_mb_s(&old, &new);
                log::append_observation("logs/characterization.csv", &new)?;

                for new_drive in &new.drives {
                    if matches!(recovery, RecoveryState::Normal) {
                        if let Some(latency) = new_drive.write_latency_ms {
                            if latency >= state.config.max_write_latency_ms {
                                launch::interrupt(&child)?;

                                println!(
                    "Emergency latency limit reached: {:.0} ms/write. Interrupting rsync.",
                    latency
                );

                                break;
                            }

                            if latency >= state.config.pause_write_latency_ms {
                                launch::stop(&child)?;

                                println!(
                                    "Pause latency reached: {:.0} ms/write. JR paused.",
                                    latency
                                );

                                recovery = RecoveryState::Recovery {
                                    deadline: std::time::Instant::now()
                                        + std::time::Duration::from_secs(30),
                                };

                                break;
                            }
                        }
                    }
                }

                for drive in &new.drives {
                    if let (Some(mc), Some(tc)) =
                        (drive.temperature_millicelsius, drive.temperature_c)
                    {
                        println!(
                            "DRV {:7}  T={:6} mC ({:4.1} C)  writes={}  sectors={}  write_ms={}",
                            drive.id.name,
                            mc,
                            tc,
                            drive.disk_stats.as_ref().map_or(0, |d| d.fields[4]),
                            drive.disk_stats.as_ref().map_or(0, |d| d.fields[6]),
                            drive.disk_stats.as_ref().map_or(0, |d| d.fields[7]),
                        );
                    }
                }
                if let Some(drive) = new.drives.first() {
                    if let Some(burst_length) =
                        burst_tracker.record_sample(drive.bytes_written, drive.write_latency_ms)
                    {
                        let verdict = assessment.record_burst(burst_length);
                        println!("Assessment: {:?}", verdict);
                    }
                }

                old = new;

                thread::sleep(Duration::from_millis(state.config.sample_interval_ms));
            }
        }
    };

    println!("rsync exited with {}", status);
    println!("PASS  algs");
    state.phase = Phase::Completed;
    println!("SUCCESS");

    Ok(())
}
