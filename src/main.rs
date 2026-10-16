mod algs;
mod assessment;
mod fleet;
mod init;
mod latency_control;
mod latency_history;
mod launch;
mod log;
mod nvme;
mod observe;
mod phase;
mod preflight;
mod probe;
mod state;
mod topology;
mod types;

use crate::phase::Phase;
use crate::state::{ActionRequest, RecoveryState, State};
use crate::types::{Capabilities, Configuration, DriveCapability, DriveId, Observation, SampleLog};
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

const SLOW_POKE_KB: u64 = 1000;
const BWLIMIT_WATCHDOG_INTERVAL: Duration = Duration::from_secs(60);

// The worst per-drive rolling average controls bandwidth; slope is diagnostic.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("RustySync {}", init::LOADLEVEL_VERSION);
    let preflight = preflight::run()?;
    println!(
        "CONTROL slow trigger=any drive rolling average>={} ms OR raw latency>={} ms; fast={} KiB/s; slow={} KiB/s; recovery average<{} ms; slope diagnostic only",
        latency_control::SLOW_LATENCY_MS,
        latency_control::EMERGENCY_LATENCY_MS,
        preflight.fast_poke_kb,
        SLOW_POKE_KB,
        latency_control::SLOW_LATENCY_MS
    );
    let nvme_names = topology::discover(&preflight.destination)?;
    println!("MONITORING  {}", nvme_names.join(", "));
    println!("SOURCE      {}", preflight.source);
    println!("DESTINATION {}", preflight.destination);
    let mut controls = fleet::Fleet::new(&nvme_names);
    let mut latency_history = latency_history::LatencyHistory::default();
    let mut canary_name = nvme_names[0].clone();

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

    // Use the selected destination for both monitoring and the transfer.
    let initial_bwlimit = format!("--bwlimit={}", preflight.fast_poke_kb);
    let mut child = launch::launch(&[
        "-aHX",
        initial_bwlimit.as_str(),
        "--numeric-ids",
        "--info=progress2",
        "--partial",
        "--whole-file",
        "--no-compress",
        "-e",
        "/usr/bin/ssh",
        "--rsync-path=sudo -n /usr/bin/rsync",
        preflight.source.as_str(),
        preflight.destination.as_str(),
    ])?;
    // Match the configured invocation limit initially. A periodic same-value
    // poke keeps sudo authorization warm between FAST/SLOW transitions.

    launch::poke_bwlimit(&child, preflight.fast_poke_kb)?;
    log::report_poke(&canary_name, true, None, None, None);
    let sample_clock = Instant::now();
    let mut fast = true;
    let mut last_bwlimit_poke = Instant::now();
    let mut progress_rate = algs::ProgressRate::new();
    let mut last_summary = Instant::now();
    let csv_path = std::env::var("RUSTYSYNC_CSV").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("{home}/.local/state/rustysync/characterization-{stamp}.csv")
    });

    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,

            None => {
                let mut new = observe::observe(&state.config, &state.capabilities);

                for new_drive in &mut new.drives {
                    if let Some(old_drive) =
                        old.drives.iter().find(|d| d.id.name == new_drive.id.name)
                    {
                        new_drive.write_latency_ms = algs::drive_latency_ms(old_drive, new_drive);
                        new_drive.bytes_written = algs::drive_bytes_written(old_drive, new_drive);
                    }
                }
                let seconds = sample_clock.elapsed().as_secs_f64();
                new.rsync_velocity_mb_s = progress_rate.sample(seconds, new.rsync_progress_bytes);
                let decision = controls.sample(seconds, &new.drives);
                let want_fast = decision.fast;
                let latency = decision.latency;
                let slope = decision.slope;
                let canary_changed = decision.canary != canary_name;
                canary_name = decision.canary;
                let mode_changed = want_fast != fast;
                if want_fast != fast {
                    launch::poke_bwlimit(
                        &child,
                        if want_fast {
                            preflight.fast_poke_kb
                        } else {
                            SLOW_POKE_KB
                        },
                    )?;
                    fast = want_fast;
                    last_bwlimit_poke = Instant::now();
                }

                if last_bwlimit_poke.elapsed() >= BWLIMIT_WATCHDOG_INTERVAL {
                    launch::poke_bwlimit(
                        &child,
                        if fast {
                            preflight.fast_poke_kb
                        } else {
                            SLOW_POKE_KB
                        },
                    )?;
                    last_bwlimit_poke = Instant::now();
                }

                // Show live status even when optional CSV logging is disabled.
                if mode_changed || canary_changed || last_summary.elapsed() >= Duration::from_secs(5) {
                    log::report_poke(
                        &canary_name,
                        fast,
                        slope,
                        latency,
                        new.rsync_velocity_mb_s,
                    );
                    last_summary = Instant::now();
                }

                if preflight.debug_log {
                    let slopes_30s = latency_history.sample(seconds, &new.drives);
                    log::append_observation(
                        &csv_path,
                        &new,
                        fast,
                        if fast {
                            preflight.fast_poke_kb
                        } else {
                            SLOW_POKE_KB
                        },
                        slope,
                        &slopes_30s,
                    )?;
                }

                old = new;

                thread::sleep(Duration::from_millis(state.config.sample_interval_ms));
            }
        }
    };

    println!("rsync exited with {}", status);
    println!("PASS  algs");
    state.phase = Phase::Completed;
    if !status.success() {
        return Err(format!("rsync failed: {status}").into());
    }
    println!("SUCCESS");

    Ok(())
}
