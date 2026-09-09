use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

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
            let mut last_bytes: Option<u64> = None;

            for chunk_result in reader.split(b'\n') {
                let chunk = match chunk_result {
                    Ok(chunk) => chunk,
                    Err(_) => break,
                };

                let text = String::from_utf8_lossy(&chunk);

                // Temporary diagnostic: lets us see exactly what rsync emits.
                eprintln!("RAW {:?}", text);

                if let Some(first) = text.split_whitespace().next() {
                    let digits: String = first.chars().filter(|c| c.is_ascii_digit()).collect();

                    if !digits.is_empty() {
                        if let Ok(bytes) = digits.parse::<u64>() {
                            if let Some(last) = last_bytes {
                                println!(
                                    "PROGRESS BYTES {}  {:.1} MB/s",
                                    bytes,
                                    bytes.saturating_sub(last) as f64 / 1_000_000.0
                                );
                            }

                            last_bytes = Some(bytes);
                            let _ = fs::write(PROGRESS_PATH, bytes.to_string());
                        }
                    }
                }
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
