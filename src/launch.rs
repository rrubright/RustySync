use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
const PROGRESS_PATH: &str = "/tmp/loadlevel-rsync-bytes";
const RSYNC_PATH: &str = "/home/richard/rsync-3.4.4/rsync";
pub fn progress_bytes() -> Option<u64> {
    let text = fs::read_to_string(PROGRESS_PATH).ok()?;
    text.trim().parse::<u64>().ok()
}

pub fn launch(args: &[&str]) -> Result<Child, Box<dyn std::error::Error>> {
        let mut child = Command::new(RSYNC_PATH)
        .args(args)
        .stdout(Stdio::piped())
        .spawn()?;

    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);

            for chunk_result in reader.split(b'\r') {
                let chunk = match chunk_result {
                    Ok(chunk) => chunk,
                    Err(_) => break,
                };

                let text = String::from_utf8_lossy(&chunk);

                if let Some(first) = text.split_whitespace().next() {
                    let digits: String = first.chars().filter(|c| c.is_ascii_digit()).collect();

                    if !digits.is_empty() {
                        eprintln!("PROGRESS BYTES {}", digits);
                        let _ = fs::write(PROGRESS_PATH, digits);
                    }
                }
            }
        });
    }
    Ok(child)
}
pub fn poke_bwlimit(
    child: &Child,
    kb_per_sec: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let command = format!("set variable bwlimit = {}", kb_per_sec);

    let status = Command::new("sudo")
        .args([
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
