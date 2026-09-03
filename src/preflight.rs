
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PreflightResult {
    pub source: String,
    pub destination: String,
    pub debug_log: bool,
}

pub fn run() -> Result<PreflightResult, Box<dyn std::error::Error>> {
    let remembered = load_last_run();

    let source_default = remembered.as_ref().map(|r| r.source.as_str());
    let destination_default = remembered.as_ref().map(|r| r.destination.as_str());

    let source = prompt_with_default("Source", source_default)?;
    let destination = prompt_with_default("Destination", destination_default)?;
    let debug_log = prompt_yes_no("Enable CSV debug logging?")?;

    validate_source(&source)?;
    validate_destination(&destination)?;

    let result = PreflightResult {
        source,
        destination,
        debug_log,
    };

    save_last_run(&result)?;

    Ok(result)
}

fn prompt_with_default(
    label: &str,
    default: Option<&str>,
) -> Result<String, io::Error> {
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

    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("source=") {
            source = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("destination=") {
            destination = Some(value.to_string());
        }
    }

    Some(PreflightResult {
        source: source?,
        destination: destination?,
        debug_log: false,
    })
}

fn save_last_run(result: &PreflightResult) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path().ok_or("cannot determine home directory")?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = format!(
        "source={}\ndestination={}\n",
        result.source,
        result.destination
    );

    fs::write(path, contents)?;

    Ok(())
}
