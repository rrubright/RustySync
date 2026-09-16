# RustySync

Rusty wraps rsync and reduces its bandwidth when storage write latency rises.
The source and destination entered at startup are used for the actual transfer.

## Monitoring

For ZFS destinations, Rusty discovers physical NVMe pool members with `zpool status -LP`.
Partition parents are resolved through sysfs. It also supports a direct NVMe filesystem.
Offline members and unsupported backing devices stop startup before rsync launches.
Cache devices and inactive spares are excluded; log and special allocation devices are monitored.

Each drive has its own ten-sample, in-memory latency window. Any drive's slow state
reduces the entire stream. Every drive that triggered slowing must recover before
FAST resumes; missing or idle samples do not count as recovery. The canary is the
highest-latency drive among latched slow drives, or among all drives when none is slow.
The slope threshold remains 10 ms/s. A valid latency reading below 1000 ms clears
a drive’s slow latch.
FAST remains 40,000 KiB/s and SLOW remains 1,000 KiB/s.

Console status appears every five seconds. Canary and rate changes appear immediately.
Speed is a rolling rsync progress estimate in MiB/s, not a physical-device write rate.
Latency comes from completed-write counters, not an end-to-end request timer.

CSV remains opt-in at each startup, default **No**. It includes every drive's
latency/counters; the explicitly named canary slope column is the shared control
snapshot. Logs use a per-run location under the user's state directory, or the split
session's run directory. `RUSTYSYNC_CSV` overrides that path.

## Top-and-bottom terminal

Build with `cargo build --offline`, install `tmux` if needed, then run:

```sh
./scripts/rusty-split
```

The top pane runs Rusty's prompts and status; the bottom follows transitions and
stderr, including warnings and errors. Enter source/destination in the top pane.
SSH password prompts may appear in the bottom pane; input belongs in the top pane.
The lower pane occupies 35% of the terminal. Codex can remain beside the terminal.
Detach using tmux's Ctrl-B then D; reattach using `tmux attach`.
Closing or detaching the terminal does not impose a time limit on the transfer.
The wrapper prints its run-log location when Rusty exits. CSV is created only if enabled.

## Existing runtime requirements

The custom rsync executable is still `/usr/local/bin/rsync-3.5.0`.
Bandwidth changes still require the existing noninteractive sudo/gdb setup and an
rsync build with its bandwidth symbol available. Remote rsync still uses
`sudo -n /usr/bin/rsync`. These must be configured before a real transfer.
The debugger-based rate change has not been validated against a live transfer here.
Do not run concurrent Rusty instances: the existing progress-counter file is shared.

## Verification

`cargo test --offline` exercises pool-member parsing, separate latency histories,
pool throttling/recovery, missing samples, and the existing telemetry/control tests.
Tests do not start rsync or write to the storage pool.
