use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};

// Shared by the reader thread and observer; concurrent runs would collide.
const PROGRESS_PATH: &str = "/tmp/loadlevel-rsync-bytes";
const RSYNC_PATH: &str = "/usr/local/bin/rsync-3.5.0";

pub fn progress_bytes() -> Option<u64> {
    let text = fs::read_to_string(PROGRESS_PATH).ok()?;
    text.trim().parse::<u64>().ok()
}

pub fn launch(args: &[&str]) -> Result<Child, Box<dyn std::error::Error>> {
    // Don't let a byte count from an earlier run masquerade as live progress.
    let _ = fs::remove_file(PROGRESS_PATH);

    let mut child = Command::new(RSYNC_PATH)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Take both handles before spawning either thread so the closures
    // do not capture or partially move `child`.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Some(stdout) = stdout {
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut chunk = Vec::new();
            // progress2 rewrites a terminal line with CR; waiting for LF can
            // hide updates for an entire file. Accept either record boundary.
            for byte in reader.bytes() {
                let byte = match byte {
                    Ok(byte) => byte,
                    Err(_) => break,
                };
                if byte == b'\r' || byte == b'\n' {
                    if let Some(bytes) = parse_progress(&chunk) {
                        let _ = fs::write(PROGRESS_PATH, bytes.to_string());
                    }
                    chunk.clear();
                } else {
                    chunk.push(byte);
                }
            }
            if let Some(bytes) = parse_progress(&chunk) {
                let _ = fs::write(PROGRESS_PATH, bytes.to_string());
            }
        });
    }

    if let Some(stderr) = stderr {
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);

            for line in reader.lines() {
                match line {
                    Ok(line) => eprintln!("RSYNC STDERR: {}", line),
                    Err(_) => break,
                }
            }
        });
    }

    Ok(child)
}

/// Attach to the launched PID and change its in-memory `bwlimit` symbol.
/// Requires noninteractive sudo/gdb access and an rsync build exposing that
/// symbol. A successful command does not verify the resulting transfer rate
/// or update any forked rsync processes.
pub fn poke_bwlimit(child: &Child, kb_per_sec: u64) -> Result<(), Box<dyn std::error::Error>> {
    let command = format!("set variable bwlimit = {}", kb_per_sec);

    let status = Command::new("sudo")
        .args([
            "-n",
            "gdb",
            "-q",
            "-batch",
            "-p",
            &child.id().to_string(),
            "-ex",
            &command,
            "-ex",
            "detach",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if !status.success() {
        return Err(format!("gdb bwlimit poke failed with {}", status).into());
    }

    Ok(())
}

pub fn stop(child: &Child) -> std::io::Result<()> {
    use nix::sys::signal::{kill, Signal::SIGSTOP};
    use nix::unistd::Pid;

    kill(Pid::from_raw(child.id() as i32), SIGSTOP).map_err(std::io::Error::other)
}

pub fn cont(child: &Child) -> std::io::Result<()> {
    use nix::sys::signal::{kill, Signal::SIGCONT};
    use nix::unistd::Pid;

    kill(Pid::from_raw(child.id() as i32), SIGCONT).map_err(std::io::Error::other)
}

pub fn interrupt(child: &Child) -> std::io::Result<()> {
    use nix::sys::signal::{kill, Signal::SIGINT};
    use nix::unistd::Pid;

    kill(Pid::from_raw(child.id() as i32), SIGINT).map_err(std::io::Error::other)
}

// Require a percentage after the byte count to reject ordinary rsync messages.
fn parse_progress(record: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(record).ok()?;
    let mut fields = text.split_whitespace();
    let count = fields.next()?;
    let percent = fields.next()?.strip_suffix('%')?.parse::<u8>().ok()?;
    if percent > 100 || !count.chars().all(|c| c.is_ascii_digit() || c == ',') {
        return None;
    }
    count.replace(',', "").parse().ok()
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    #[test]
    fn parses_progress_and_rejects_diagnostics() {
        assert_eq!(parse_progress(b" 1,234 10% 2.0MB/s"), Some(1234));
        assert_eq!(parse_progress(b" 0 0% 0.0kB/s"), Some(0));
        assert_eq!(parse_progress(b"123 files to consider"), None);
        assert_eq!(parse_progress(b""), None);
    }
}
