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
mod latency_control;
mod topology;
mod fleet;
mod state;
mod types;

use crate::phase::Phase;
use crate::state::{ActionRequest, RecoveryState, State};
use crate::types::{Capabilities, Configuration, DriveCapability, DriveId, Observation, SampleLog};
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

const SLOW_POKE_KB: u64 = 1000;
const FAST_POKE_KB: u64 = 40_000;

// Rusty slows on the slope of smoothed latency and recovers on a valid
// sub-millisecond reading. Missing latency never clears the SLOW state.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("RustySync {}", init::LOADLEVEL_VERSION);
    println!("CONTROL slow trigger={} ms/s; fast={} KiB/s; slow={} KiB/s; recovery latency<1 ms",
        latency_control::SLOW_SLOPE_MS_PER_SEC, FAST_POKE_KB, SLOW_POKE_KB);
    let preflight = preflight::run()?;
    let nvme_names = topology::discover(&preflight.destination)?;
    println!("MONITORING  {}", nvme_names.join(", "));
    println!("SOURCE      {}", preflight.source);
    println!("DESTINATION {}", preflight.destination);
    let mut controls = fleet::Fleet::new(&nvme_names);
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
        preflight.source.as_str(),
        preflight.destination.as_str(),
    ])?;
    // Match the invocation's 40,000 limit initially. Subsequent pokes happen
    // only when the FAST/SLOW state changes; there is no periodic reassertion.

    launch::poke_bwlimit(&child, FAST_POKE_KB)?;
    log::report_poke("POKE INITIAL", &canary_name, true, FAST_POKE_KB, None, None, None);
    let sample_clock = Instant::now();
    let mut fast = true;
    let mut progress_rate = algs::ProgressRate::new();
    let mut last_summary = Instant::now();
    let csv_path = std::env::var("RUSTYSYNC_CSV").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_secs();
        format!("{home}/.local/state/rustysync/characterization-{stamp}.csv")
    });

    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,

            None => {
                let mut new = observe::observe(&state.config, &state.capabilities);

                for new_drive in &mut new.drives {
                    if let Some(old_drive) = old.drives.iter().find(|d| d.id.name == new_drive.id.name) {
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
                if decision.canary != canary_name {
                    canary_name = decision.canary;
                    log::report_poke("CANARY", &canary_name, fast,
                        if fast { FAST_POKE_KB } else { SLOW_POKE_KB },
                        slope, latency, new.rsync_velocity_mb_s);
                }
                if want_fast != fast {
                    launch::poke_bwlimit(&child, if want_fast { FAST_POKE_KB } else { SLOW_POKE_KB })?;
                    fast = want_fast;
                    log::report_poke("POKE CHANGE", &canary_name, fast,
                        if fast { FAST_POKE_KB } else { SLOW_POKE_KB }, slope, latency, new.rsync_velocity_mb_s);
                    last_summary = Instant::now();
                }

                // Show live status even when optional CSV logging is disabled.
                if last_summary.elapsed() >= Duration::from_secs(5) {
                    log::report_poke("STATUS", &canary_name, fast,
                        if fast { FAST_POKE_KB } else { SLOW_POKE_KB },
                        slope, latency, new.rsync_velocity_mb_s);
                    last_summary = Instant::now();
                }

                if preflight.debug_log {
                    log::append_observation(&csv_path, &new, fast,
                        if fast { FAST_POKE_KB } else { SLOW_POKE_KB }, slope)?;
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
