use std::fs;

pub fn temperature_millicelsius(device: &str) -> Result<i32, String> {
    let block_name = device
        .rsplit('/')
        .next()
        .ok_or_else(|| format!("bad device path: {}", device))?;

    let controller = block_name
        .split_once("n1")
        .map(|(controller, _)| controller)
        .ok_or_else(|| format!("bad nvme block name: {}", block_name))?;

    let base = format!("/sys/class/nvme/{}", controller);

    let entries = fs::read_dir(&base).map_err(|e| format!("cannot read {}: {}", base, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("nvme entry error: {}", e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if name.starts_with("hwmon") {
            let temp_path = entry.path().join("temp1_input");

            if temp_path.exists() {
                let raw = fs::read_to_string(&temp_path)
                    .map_err(|e| format!("cannot read {:?}: {}", temp_path, e))?;

                let milli_c: i32 = raw
                    .trim()
                    .parse()
                    .map_err(|e| format!("cannot parse {:?}: {}", temp_path, e))?;

                return Ok(milli_c);
            }
        }
    }

    Err(format!("no hwmon temp1_input found under {}", base))
}
