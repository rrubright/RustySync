use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_FAST_POKE_KB: u64 = 55_000;

#[derive(Debug, Clone)]
pub struct PreflightResult {
    pub source: String,
    pub destination: String,
    pub fast_poke_kb: u64,
    pub debug_log: bool,
}

pub fn run() -> Result<PreflightResult, Box<dyn std::error::Error>> {
    let remembered = load_last_run();

    let source_default = remembered.as_ref().map(|r| r.source.as_str());
    let destination_default = remembered.as_ref().map(|r| r.destination.as_str());
    let fast_poke_default = remembered
        .as_ref()
        .map_or(DEFAULT_FAST_POKE_KB, |r| r.fast_poke_kb);

    let source = prompt_with_default("Source", source_default)?;
    let destination = prompt_with_default("Destination", destination_default)?;
    let fast_poke_kb = prompt_positive_u64("Fast bandwidth KiB/s", fast_poke_default)?;
    let debug_log = prompt_yes_no("Enable CSV debug logging?")?;

    validate_source(&source)?;
    validate_destination(&destination)?;
    let result = PreflightResult {
        source,
        destination,
        fast_poke_kb,
        debug_log,
    };

    save_last_run(&result)?;

    Ok(result)
}
pub fn backing_device(destination: &str) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", "--target", destination])
        .output()?;

    if !output.status.success() {
        return Err(format!("could not resolve backing device for {destination}").into());
    }

    let source = String::from_utf8(output.stdout)?.trim().to_string();

    if source.is_empty() {
        return Err(format!("no backing device found for {destination}").into());
    }

    Ok(source)
}
fn prompt_with_default(label: &str, default: Option<&str>) -> Result<String, io::Error> {
    match default {
        Some(value) => print!("{label} [{value}]: "),
        None => print!("{label}: "),
    }

    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let input = input.trim();

    if input.is_empty() {
        if let Some(value) = default {
            return Ok(value.to_string());
        }
    }

    Ok(input.to_string())
}

fn prompt_yes_no(label: &str) -> Result<bool, io::Error> {
    print!("{label} [y/N]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn prompt_positive_u64(label: &str, default: u64) -> Result<u64, io::Error> {
    loop {
        print!("{label} [{default}]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            return Ok(default);
        }

        match input.parse::<u64>() {
            Ok(value) if value > 0 => return Ok(value),
            _ => eprintln!("Enter a positive whole number of KiB/s."),
        }
    }
}

fn validate_source(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    if source.trim().is_empty() {
        return Err("source cannot be empty".into());
    }

    Ok(())
}

fn validate_destination(destination: &str) -> Result<(), Box<dyn std::error::Error>> {
    if destination.trim().is_empty() {
        return Err("destination cannot be empty".into());
    }

    let path = Path::new(destination);

    if !path.exists() {
        return Err(format!("destination does not exist: {destination}").into());
    }

    if !path.is_dir() {
        return Err(format!("destination is not a directory: {destination}").into());
    }

    Ok(())
}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;

    Some(
        PathBuf::from(home)
            .join(".config")
            .join("rustysync")
            .join("last_run"),
    )
}

fn load_last_run() -> Option<PreflightResult> {
    let path = config_path()?;
    let contents = fs::read_to_string(path).ok()?;

    let mut source = None;
    let mut destination = None;
    let mut fast_poke_kb = None;

    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("source=") {
            source = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("destination=") {
            destination = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("fast_poke_kb=") {
            fast_poke_kb = value.parse::<u64>().ok().filter(|value| *value > 0);
        }
    }

    Some(PreflightResult {
        source: source?,
        destination: destination?,
        fast_poke_kb: fast_poke_kb.unwrap_or(DEFAULT_FAST_POKE_KB),
        debug_log: false,
    })
}

fn save_last_run(result: &PreflightResult) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path().ok_or("cannot determine home directory")?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = format!(
        "source={}\ndestination={}\nfast_poke_kb={}\n",
        result.source, result.destination, result.fast_poke_kb
    );

    fs::write(path, contents)?;

    Ok(())
}
