use crate::{latency_control::LatencyControl, types::DriveSample};

pub struct Fleet { controls: Vec<(String, LatencyControl)> }
pub struct Decision {
    pub fast: bool,
    pub canary: String,
    pub latency: Option<f64>,
    pub slope: Option<f64>,
}
impl Fleet {
    pub fn new(names: &[String]) -> Self {
        Self { controls: names.iter().map(|n| (n.clone(), LatencyControl::new())).collect() }
    }
    pub fn sample(&mut self, seconds: f64, drives: &[DriveSample]) -> Decision {
        let mut rows = Vec::new();
        for (name, control) in &mut self.controls {
            let latency = drives.iter().find(|d| &d.id.name == name)
                .and_then(|d| d.write_latency_ms).filter(|v| v.is_finite() && *v >= 0.0);
            let (fast, slope) = control.sample(seconds, latency);
            rows.push((name.clone(), fast, latency, slope));
        }
        let fast = rows.iter().all(|r| r.1);
        // A latched slow drive takes precedence; missing samples cannot recover it.
        let selected = rows.into_iter().max_by(|a, b| {
            (!a.1).cmp(&(!b.1)).then_with(||
                a.2.unwrap_or(-1.0).total_cmp(&b.2.unwrap_or(-1.0)))
        }).expect("Fleet requires at least one drive");
        Decision { fast, canary: selected.0, latency: selected.2, slope: selected.3 }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DriveId;
    fn drive(name: &str, latency: Option<f64>) -> DriveSample {
        DriveSample { id: DriveId { name: name.into() }, temperature_millicelsius: None,
            temperature_c: None, disk_stats: None, write_latency_ms: latency, bytes_written: None }
    }
    #[test]
    fn any_drive_slows_pool_and_missing_drive_cannot_clear_it() {
        let mut fleet = Fleet::new(&["a".into(), "b".into()]);
        for tick in 0..25 {
            let t = tick as f64 * 0.1;
            fleet.sample(t, &[drive("a", Some(0.2)), drive("b", Some(10.0 + 20.0*t))]);
        }
        let result = fleet.sample(3.0, &[drive("a", Some(0.2))]);
        assert!(!result.fast);
        assert_eq!(result.canary, "b");
        assert!(fleet.sample(3.1, &[drive("a", Some(0.2)), drive("b", Some(0.5))]).fast);
    }
    #[test]
    fn different_baselines_do_not_create_trends() {
        let mut fleet = Fleet::new(&["a".into(), "b".into()]);
        for tick in 0..30 {
            // Reordered samples still match the correct drive history.
            let result = fleet.sample(tick as f64 * 0.1,
                &[drive("b", Some(100.0)), drive("a", Some(0.2))]);
            assert!(result.fast);
            assert_eq!(result.canary, "b");
        }
    }
}
