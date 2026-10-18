//! NPU utilisation from the Intel `ivpu` driver's busy-time counter.

use std::fs;
use std::path::Path;
use std::time::Instant;

/// Cumulative NPU busy time in microseconds, if an `ivpu` device exposes it.
pub fn busy_us(sysfs_class_accel: &Path) -> Option<u64> {
    let mut entries: Vec<_> = fs::read_dir(sysfs_class_accel).ok()?.flatten().collect();
    // Stable order, so two NPUs don't swap places between polls.
    entries.sort_by_key(|e| e.file_name());
    entries.iter().find_map(|entry| {
        fs::read_to_string(entry.path().join("device/npu_busy_time_us"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    })
}

/// Turns the counter into a percentage between polls.
pub struct Meter {
    last: Option<(u64, Instant)>,
}

impl Meter {
    pub fn new() -> Self {
        Self { last: None }
    }

    /// Feed the counter as read at `now`. The first reading, and any reading
    /// after the counter went missing or reset, has nothing to compare with
    /// and yields `None`.
    pub fn update(&mut self, reading: Option<u64>, now: Instant) -> Option<f64> {
        let reading = match reading {
            Some(r) => r,
            None => {
                self.last = None;
                return None;
            }
        };
        let percent = self.last.and_then(|(prev, then)| {
            // A driver reload restarts the counter from zero.
            let busy = reading.checked_sub(prev)?;
            let wall = now.duration_since(then).as_micros().max(1) as f64;
            Some((busy as f64 / wall * 100.0).min(100.0))
        });
        self.last = Some((reading, now));
        percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn needs_two_readings() {
        let t0 = Instant::now();
        let mut meter = Meter::new();
        assert_eq!(meter.update(Some(1_000), t0), None);
        let got = meter
            .update(Some(1_500_000), t0 + Duration::from_secs(2))
            .unwrap();
        assert!((got - 74.95).abs() < 0.01, "{got}");
    }

    #[test]
    fn caps_at_100() {
        let t0 = Instant::now();
        let mut meter = Meter::new();
        meter.update(Some(0), t0);
        assert_eq!(
            meter.update(Some(9_000_000), t0 + Duration::from_secs(1)),
            Some(100.0)
        );
    }

    #[test]
    fn a_counter_that_appears_late_still_works() {
        let t0 = Instant::now();
        let mut meter = Meter::new();
        assert_eq!(meter.update(None, t0), None);
        assert_eq!(meter.update(Some(10), t0 + Duration::from_secs(1)), None);
        assert!(
            meter
                .update(Some(500_010), t0 + Duration::from_secs(2))
                .is_some()
        );
    }

    #[test]
    fn a_reset_counter_is_not_reported() {
        let t0 = Instant::now();
        let mut meter = Meter::new();
        meter.update(Some(5_000_000), t0);
        assert_eq!(meter.update(Some(100), t0 + Duration::from_secs(1)), None);
        assert!(
            meter
                .update(Some(500_100), t0 + Duration::from_secs(2))
                .is_some()
        );
    }

    #[test]
    fn reads_the_counter_from_sysfs() {
        let root = std::env::temp_dir().join(format!("vinowatch-npu-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        assert_eq!(busy_us(&root), None);
        fs::create_dir_all(root.join("accel0/device")).unwrap();
        assert_eq!(busy_us(&root), None);
        fs::write(root.join("accel0/device/npu_busy_time_us"), "123456\n").unwrap();
        assert_eq!(busy_us(&root), Some(123456));
        fs::remove_dir_all(&root).unwrap();
    }
}
