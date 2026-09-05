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
mod preflight;

use crate::phase::Phase;
use crate::state::{ActionRequest, RecoveryState, State};
use crate::types::{Capabilities, Configuration, DriveCapability, DriveId, Observation, SampleLog};
use std::fs;
use std::thread;
use std::time::Duration;

const CONTROL_DRIVE: &str = "nvme1n1";
const DEFAULT_BWLIMIT_KB: u64 = 20;
const NORMAL_BWLIMIT_KB: u64 = 40_000;
const RECOVERY_TRIGGER_MS: f64 = 200.0;
const QUIET_SAMPLES: u32 = 5;// Main control loop.
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
    println!("RustySync {}", init::LOADLEVEL_VERSION);
    let preflight = preflight::run()?;

    println!("SOURCE      {}", preflight.source);
    println!("DESTINATION {}", preflight.destination);
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
        action_request: ActionRequest::Continue,
    };

    let mut recovery = RecoveryState::Normal;

    init::init()?;

    println!("PASS  init");

    state.phase = Phase::Calibrating;

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
        //         // temperature sensing, and missing-data detection before the
        // runtime rsync observation loop was implemented.
        //
        // The runtime loop below is now the primary observation engine.
        // This block is retained for regression testing and may later
        // become a dedicated self-test mode.

        for _ in 0..state.config.calibration_samples {
            let observation = observe::observe(&state.config, &state.capabilities);

            for flag in &observation.missing_flags {
                *missing_counts.entry(format!("{:?}", flag)).or_insert(0) += 1;
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
    "-aHX",
    "--bwlimit=20",
    "--numeric-ids",
    "--info=progress2",
    "--partial",
    "--whole-file",
    "--no-compress",
    "-e",
    "/usr/bin/ssh",
     "--rsync-path=sudo -n /usr/bin/rsync",
    "richard@192.168.1.176:/media/richard/Shareable/NVNas_Backup/",
    "/srv/storage/",
    ])?;
// rsync starts at a conservative 20 KB/s fallback.
// RustySync actively raises it to 40,000 KB/s for Normal operation.
//
// This is intentional. If rsync ever restores its invocation-time
// bwlimit on its own, it falls back safely to 20 KB/s.
//
// Any write sample at or above RECOVERY_TRIGGER_MS forces 20 KB/s
// and Recovery. Five consecutive no-write samples force 40,000 KB/s
// and Normal, regardless of the current logical state. That quiet-window
// repoke also repairs any silent fallback to the invocation-time limit.

launch::poke_bwlimit(&child, NORMAL_BWLIMIT_KB)?;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,

        None => {
                let mut new = observe::observe(&state.config, &state.capabilities);
                for (old_drive, new_drive) in old.drives.iter().zip(new.drives.iter_mut()) {
                    new_drive.write_latency_ms = algs::drive_latency_ms(old_drive, new_drive);

                  new_drive.bytes_written = algs::drive_bytes_written(old_drive, new_drive);
                }
                if let Some(drive) = new.drives.iter().find(|d| d.id.name == CONTROL_DRIVE) {
                        if let Some(bytes) = drive.bytes_written.filter(|&b| b > 0) {

                      println!(
                            "==========> WRITE control-drive  bytes={}  latency={}",
                            bytes,
                            drive

                              .write_latency_ms
                                .map(|v| format!("{:.3} ms/write", v))
                                .unwrap_or_else(|| "-".to_string())
                        );

                  }
                }
                if let Some(drive) =
                    new.drives.iter().find(|d| d.id.name == CONTROL_DRIVE)
                {
                    if let Some(latency) = drive.write_latency_ms {
                        if latency >= RECOVERY_TRIGGER_MS {
                            launch::poke_bwlimit(&child, DEFAULT_BWLIMIT_KB)?;
                            recovery = RecoveryState::Recovery { ticks: 0 };

                            println!(
                                "{} RECOVERY  latency={:.0} ms  bwlimit={} KB/s",
                                "=".repeat(80),
                                latency,
                                DEFAULT_BWLIMIT_KB
                            );
                        }
                    }
                }

                match recovery {
                    RecoveryState::Normal => {
                }
                    RecoveryState::Recovery { ticks } => {
                        if let Some(drive) =

                          new.drives.iter().find(|d| d.id.name == CONTROL_DRIVE)
                        {
                            let writing =
                                drive.bytes_written.is_some_and(|bytes| bytes > 0);


                           if writing {
                                recovery = RecoveryState::Recovery { ticks: 0 };
                            } else {
                                let empty_samples = ticks + 1;

                                  println!(
                                        "{} RECOVERY  no-write {}/{}",
                                        "=".repeat(80),
                                        empty_samples,

                                     QUIET_SAMPLES
                                );
                                if empty_samples >= QUIET_SAMPLES {
                                    launch::poke_bwlimit(&child, NORMAL_BWLIMIT_KB)?;
                                  recovery = RecoveryState::Normal;

                                println!(
                                    "{} NORMAL  bwlimit={} KB/s",

                                  "=".repeat(80),
                                    NORMAL_BWLIMIT_KB
                                );
                                } else {

                                  recovery = RecoveryState::Recovery {
                                        ticks: empty_samples,
                                    };
                                }

                          }
                        }
                    }


                  RecoveryState::Probing => {}
                }

                for drive in &new.drives {

                  if let (Some(mc), Some(tc)) =
                        (drive.temperature_millicelsius, drive.temperature_c)
                    {
                       

              }
            }
                if let Some(drive) = new.drives.iter().find(|d| d.id.name == CONTROL_DRIVE) {
                    if let Some(burst_length) =

                      burst_tracker.record_sample(drive.bytes_written, drive.write_latency_ms)
                    {
                        let verdict = assessment.record_burst(burst_length);
                  }
                }
                log::append_observation("/src/logs/characterization.csv", &new)?;
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

