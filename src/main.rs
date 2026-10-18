use std::collections::HashMap;
use std::fs;
use std::thread::sleep;
use std::time::{Duration, Instant};
use sysinfo::{ProcessesToUpdate, System};

/// Returns true if the process looks like an OpenVINO user (by name or loaded libraries).
fn uses_openvino(pid: u32, name: &str) -> bool {
    if name.to_lowercase().contains("openvino") {
        return true;
    }
    fs::read_to_string(format!("/proc/{pid}/maps"))
        .map(|m| m.lines().any(|l| l.to_lowercase().contains("libopenvino")))
        .unwrap_or(false)
}

/// Cumulative NPU busy time in microseconds (Intel ivpu driver), if available.
fn npu_busy_us() -> Option<u64> {
    for entry in fs::read_dir("/sys/class/accel").ok()?.flatten() {
        let p = entry.path().join("device/npu_busy_time_us");
        if let Some(v) = fs::read_to_string(p).ok().and_then(|s| s.trim().parse().ok()) {
            return Some(v);
        }
    }
    None
}

fn main() {
    let interval = Duration::from_secs(
        std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(2),
    );
    let mut sys = System::new_all();
    let mut last_npu: Option<(u64, Instant)> = npu_busy_us().map(|v| (v, Instant::now()));
    println!("vinoWatch: monitoring OpenVINO processes every {}s", interval.as_secs());
    loop {
        sleep(interval);
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let mut found: HashMap<u32, (String, f32, u64)> = HashMap::new();
        for (pid, p) in sys.processes() {
            let name = p.name().to_string_lossy().to_string();
            if uses_openvino(pid.as_u32(), &name) {
                found.insert(pid.as_u32(), (name, p.cpu_usage(), p.memory()));
            }
        }
        let npu = match (npu_busy_us(), last_npu) {
            (Some(now), Some((prev, t))) => {
                let el = t.elapsed().as_micros().max(1) as f64;
                last_npu = Some((now, Instant::now()));
                Some((now.saturating_sub(prev) as f64 / el * 100.0).min(100.0))
            }
            _ => None,
        };
        match npu {
            Some(n) => println!("NPU: {n:.1}%"),
            None => println!("NPU: unavailable"),
        }
        if found.is_empty() {
            println!("  no OpenVINO processes found");
        }
        for (pid, (name, cpu, mem)) in &found {
            println!("  {name} (pid {pid}): CPU {cpu:.1}%, RAM {} MiB", mem / 1048576);
        }
    }
}
