use std::{collections::BTreeSet, path::Path, process::Command};
type Error = Box<dyn std::error::Error>;

fn output(command: &str, args: &[&str]) -> Result<String, Error> {
    let result = Command::new(command).args(args).output()?;
    if !result.status.success() {
        return Err(format!("{command} failed: {}", String::from_utf8_lossy(&result.stderr)).into());
    }
    Ok(String::from_utf8(result.stdout)?.trim().into())
}

pub fn discover(destination: &str) -> Result<Vec<String>, Error> {
    let source = crate::preflight::backing_device(destination)?;
    let kind = output("findmnt", &["-n", "-o", "FSTYPE", "--target", destination])?;
    let paths = if kind == "zfs" {
        let pool = source.split('/').next().ok_or("missing pool name")?;
        pool_paths(&output("zpool", &["status", "-LP", pool])?)?
    } else {
        vec![source]
    };
    let mut names = BTreeSet::new();
    for path in paths {
        let canonical = std::fs::canonicalize(&path)?;
        let name = canonical.file_name().and_then(|s| s.to_str()).ok_or("invalid device name")?;
        // Resolve partition parents through sysfs, without assuming partition 1.
        let sys = std::fs::canonicalize(format!("/sys/class/block/{name}"))?;
        let whole = if sys.join("partition").exists() {
            sys.parent().and_then(Path::file_name).and_then(|s| s.to_str()).ok_or("missing partition parent")?
        } else { name };
        if !whole.starts_with("nvme") || !Path::new(&format!("/sys/class/block/{whole}/device")).exists() {
            return Err(format!("unsupported backing device: {whole}; expected physical NVMe").into());
        }
        names.insert(whole.to_owned());
    }
    if names.is_empty() { return Err("no pool drives found".into()); }
    Ok(names.into_iter().collect())
}

fn pool_paths(status: &str) -> Result<Vec<String>, Error> {
    let mut config = false;
    let mut auxiliary = false;
    let mut paths = Vec::new();
    for line in status.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let Some(&name) = fields.first() else { continue };
        if name == "config:" { config = true; continue; }
        if name == "errors:" { break; }
        if !config { continue; }
        if matches!(name, "cache" | "spares") { auxiliary = true; continue; }
        if matches!(name, "logs" | "special" | "dedup") { auxiliary = false; continue; }
        if auxiliary || name == "NAME" { continue; }
        if fields.len() >= 2 && fields[1] != "ONLINE" {
            return Err(format!("pool member {name} is not ONLINE").into());
        }
        if name.starts_with("/dev/") { paths.push(name.to_string()); }
    }
    if paths.is_empty() { return Err("no physical pool members found".into()); }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_four_members_and_ignores_cache() {
        let s = "config:\n NAME STATE READ WRITE CKSUM\n tank ONLINE 0 0 0\n raidz2-0 ONLINE 0 0 0\n /dev/nvme0n1p1 ONLINE 0 0 0\n /dev/nvme1n1p1 ONLINE 0 0 0\n /dev/nvme2n1p1 ONLINE 0 0 0\n /dev/nvme3n1p1 ONLINE 0 0 0\n cache\n /dev/nvme4n1 ONLINE 0 0 0\nerrors: No known data errors";
        assert_eq!(pool_paths(s).unwrap().len(), 4);
    }
    #[test]
    fn refuses_missing_or_offline_members() {
        assert!(pool_paths("config:\n /dev/nvme0n1 OFFLINE 0 0 0").is_err());
        assert!(pool_paths("permission denied").is_err());
    }
}
