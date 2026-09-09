mod algs;
mod assessment;
mod init;
mod launch;
mod log;
mod nvme;
mod observe;
mod phase;
mod preflight;
mod probe;
mod state;
mod types;

use crate::phase::Phase;
use crate::state::{ActionRequest, RecoveryState, State};
use crate::types::{Capabilities, Configuration, DriveCapability, DriveId, Observation, SampleLog};
use std::fs;
use std::thread;
use std::time::Duration;

const SLOW_POKE_KB: u64 = 1000;
const FAST_POKE_KB: u64 = 40_000;
const WINDOW_TICKS: u32 = 10;
const WRITES_PER_WINDOW_LIMIT: u64 = 50;
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
    let backing_device = preflight::backing_device(&preflight.destination)?;
    let backing_name = std::path::Path::new(&backing_device)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("could not extract backing device name")?;

    let backing_nvme = backing_name
        .strip_suffix("p1")
        .ok_or("destination is not a simple NVMe partition")?
        .to_string();
    println!("BACKING     {} -> {}", backing_device, backing_nvme);
    println!("SOURCE      {}", preflight.source);
    println!("DESTINATION {}", preflight.destination);

    // Previously discovered all NVMe devices in /sys/class/block.
    // For now, observe only the NVMe backing /srv/storage.
    // Later, replace this with destination-topology discovery for RAID/LVM.

    let nvme_names = vec![backing_nvme.clone()];

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
        "--bwlimit=40000",
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

    launch::poke_bwlimit(&child, FAST_POKE_KB)?;
    let mut window_ticks: u32 = 0;
    let mut fast = true;
    let mut window_start_writes: Option<u64> = None;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,

            None => {
                let mut new = observe::observe(&state.config, &state.capabilities);

                for (old_drive, new_drive) in old.drives.iter().zip(new.drives.iter_mut()) {
                    new_drive.write_latency_ms = algs::drive_latency_ms(old_drive, new_drive);
                    new_drive.bytes_written = algs::drive_bytes_written(old_drive, new_drive);
                }
                let canary = new
                    .drives
                    .iter()
                    .filter(|d| d.id.name == backing_nvme)
                    .max_by(|a, b| {
                        a.write_latency_ms
                            .unwrap_or(0.0)
                            .partial_cmp(&b.write_latency_ms.unwrap_or(0.0))
                            .unwrap()
                    });
                if let Some(drive) = canary.as_ref() {
                    if let Some(bytes) = drive.bytes_written.filter(|&b| b > 0) {
                        println!(
                            "==========> {}  {}={} KB/s  bytes={}  latency={}",
                            drive.id.name,
                            if fast { "FAST_POKE" } else { "SLOW_POKE" },
                            if fast { FAST_POKE_KB } else { SLOW_POKE_KB },
                            bytes,
                            drive
                                .write_latency_ms
                                .map(|v| format!("{:.3} ms/write", v))
                                .unwrap_or_else(|| "-".to_string())
                        );
                    }
                }

                if let Some(drive) = canary.as_ref() {
                    if let Some(current_writes) = drive
                        .disk_stats
                        .as_ref()
                        .and_then(|stats| stats.fields.get(4).copied())
                    {
                        if window_start_writes.is_none() {
                            window_start_writes = Some(current_writes);
                        }

                        window_ticks += 1;

                        if window_ticks >= WINDOW_TICKS {
                            let writes_in_window =
                                current_writes.saturating_sub(window_start_writes.unwrap());
                            println!(
                                "==========> WINDOW  drive={}  state={}  writes={}  latency={}",
                                drive.id.name,
                                if fast { "FAST" } else { "SLOW" },
                                writes_in_window,
                                drive
                                    .write_latency_ms
                                    .map(|v| format!("{:.3} ms/write", v))
                                    .unwrap_or_else(|| "-".to_string())
                            );
                            let want_fast = if fast {
                                writes_in_window > WRITES_PER_WINDOW_LIMIT
                            } else {
                                writes_in_window > WRITES_PER_WINDOW_LIMIT
                                    && drive.write_latency_ms.is_some_and(|ms| ms < 200.0)
                            };
                            if want_fast != fast {
                                if want_fast {
                                    launch::poke_bwlimit(&child, FAST_POKE_KB)?;
                                } else {
                                    launch::poke_bwlimit(&child, SLOW_POKE_KB)?;
                                }

                                fast = want_fast;
                            }

                            window_ticks = 0;
                            window_start_writes = Some(current_writes);
                        }
                    }
                }

                if preflight.debug_log {
                    log::append_observation("/src/logs/characterization.csv", &new)?;
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
