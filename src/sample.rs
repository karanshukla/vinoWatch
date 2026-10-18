//! One poll of the machine: which processes use OpenVINO, and the NPU load.

use std::path::{Path, PathBuf};
use std::time::Instant;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::detect::{self, Project};
use crate::npu::{self, Meter};

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub project: Option<Project>,
    pub cpu_percent: f32,
    /// PSS when readable, else resident size (see `ram_is_pss`).
    pub ram_bytes: u64,
    /// Shared pages are split between their users, so these add up. Resident
    /// size, the fallback, counts them in every process.
    pub ram_is_pss: bool,
    /// Has an `/dev/accel` node open, so it is the one talking to the NPU.
    pub npu_open: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// System-wide; the driver has no per-process counter. `None` when there
    /// is no `ivpu` device or this is the first poll.
    pub npu_percent: Option<f64>,
    pub procs: Vec<Proc>,
    /// `(total, available)` bytes of system RAM.
    pub mem: Option<(u64, u64)>,
    /// Available RAM is under the `--warn-below` share of the total.
    pub low_memory: bool,
}

pub struct Sampler {
    sys: System,
    meter: Meter,
    proc_root: PathBuf,
    accel_root: PathBuf,
    warn_below_percent: u8,
}

impl Sampler {
    pub fn new(warn_below_percent: u8) -> Self {
        let mut sampler = Self::with_roots("/proc".into(), "/sys/class/accel".into());
        sampler.warn_below_percent = warn_below_percent;
        sampler
    }

    pub fn with_roots(proc_root: PathBuf, accel_root: PathBuf) -> Self {
        Self {
            sys: System::new(),
            meter: Meter::new(),
            proc_root,
            accel_root,
            warn_below_percent: 0,
        }
    }

    /// CPU % is a delta, so the first poll reads 0 for everything. Call this
    /// once, wait, then poll for real.
    pub fn poll(&mut self) -> Snapshot {
        let kind = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet);
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

        let mut procs: Vec<Proc> = self
            .sys
            .processes()
            .iter()
            .filter_map(|(pid, p)| {
                let pid = pid.as_u32();
                let name = p.name().to_string_lossy().into_owned();
                let cmdline: Vec<String> = p
                    .cmd()
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                // sysinfo lists every thread as a process of its own; the
                // process is the thread group leader.
                if p.thread_kind().is_some() {
                    return None;
                }
                // Kernel threads have no command line and no libraries.
                if cmdline.is_empty() {
                    return None;
                }
                let by_name = name.to_lowercase().contains("openvino");
                if !by_name && !detect::uses_openvino(&self.proc_root, pid) {
                    return None;
                }
                let cwd = p.cwd().map(Path::to_string_lossy);
                let pss = detect::pss_bytes(&self.proc_root, pid);
                Some(Proc {
                    pid,
                    ram_bytes: pss.unwrap_or_else(|| p.memory()),
                    ram_is_pss: pss.is_some(),
                    project: detect::classify(&cmdline, cwd.as_deref()),
                    name,
                    cpu_percent: p.cpu_usage(),
                    npu_open: detect::holds_npu(&self.proc_root, pid),
                })
            })
            .collect();
        // Projects first (in a fixed order), then the busiest.
        procs.sort_by(|a, b| {
            a.project
                .is_none()
                .cmp(&b.project.is_none())
                .then(a.project.cmp(&b.project))
                .then(b.cpu_percent.total_cmp(&a.cpu_percent))
                .then(a.pid.cmp(&b.pid))
        });

        let npu_percent = self
            .meter
            .update(npu::busy_us(&self.accel_root), Instant::now());
        let mem = detect::meminfo(&self.proc_root);
        let low_memory = mem.is_some_and(|(total, available)| {
            available.saturating_mul(100) < total.saturating_mul(self.warn_below_percent.into())
        });
        Snapshot {
            npu_percent,
            procs,
            mem,
            low_memory,
        }
    }
}

impl Snapshot {
    /// One line per poll, for scripts: `jq` reads it, nothing else does.
    pub fn to_json(&self) -> String {
        let npu = match self.npu_percent {
            Some(p) => format!("{p:.1}"),
            None => "null".into(),
        };
        let procs: Vec<String> = self
            .procs
            .iter()
            .map(|p| {
                let project = match p.project {
                    Some(project) => format!("\"{project}\""),
                    None => "null".into(),
                };
                format!(
                    "{{\"pid\":{},\"name\":{},\"project\":{},\"cpu_percent\":{:.1},\"ram_mib\":{},\"ram_kind\":\"{}\",\"npu_open\":{}}}",
                    p.pid,
                    json_string(&p.name),
                    project,
                    p.cpu_percent,
                    p.ram_bytes / (1024 * 1024),
                    if p.ram_is_pss { "pss" } else { "rss" },
                    p.npu_open,
                )
            })
            .collect();
        let mib = |b: u64| b / (1024 * 1024);
        let (total, available) = match self.mem {
            Some((t, a)) => (mib(t).to_string(), mib(a).to_string()),
            None => ("null".into(), "null".into()),
        };
        format!(
            "{{\"npu_percent\":{npu},\"ram_openvino_mib\":{},\"ram_total_mib\":{total},\"ram_available_mib\":{available},\"low_memory\":{},\"processes\":[{}]}}",
            mib(self.openvino_bytes()),
            self.low_memory,
            procs.join(",")
        )
    }

    /// What the listed processes hold between them. Exact with PSS; with the
    /// resident-size fallback it can count shared pages more than once.
    pub fn openvino_bytes(&self) -> u64 {
        self.procs.iter().map(|p| p.ram_bytes).sum()
    }

    /// `OpenVINO apps hold 2.1 GiB of 7.6 GiB RAM, 4.9 GiB available`
    pub fn ram_label(&self) -> String {
        let gib = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);
        let used = gib(self.openvino_bytes());
        match self.mem {
            Some((total, available)) => format!(
                "OpenVINO apps hold {used:.1} GiB of {:.1} GiB RAM, {:.1} GiB available{}",
                gib(total),
                gib(available),
                if self.low_memory { " (low)" } else { "" },
            ),
            None => format!("OpenVINO apps hold {used:.1} GiB RAM"),
        }
    }

    pub fn npu_label(&self) -> String {
        match self.npu_percent {
            Some(p) => format!("NPU {p:.0}%"),
            None => "NPU unavailable".into(),
        }
    }
}

impl Proc {
    pub fn label(&self) -> String {
        let who = match self.project {
            Some(project) => format!("{project} ({})", self.name),
            None => self.name.clone(),
        };
        let npu = if self.npu_open { ", NPU open" } else { "" };
        format!(
            "{who} pid {}: CPU {:.1}%, RAM {} MiB{npu}",
            self.pid,
            self.cpu_percent,
            self.ram_bytes / (1024 * 1024),
        )
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escapes_what_a_process_name_can_contain() {
        assert_eq!(json_string("a\"b\\c\nd"), r#""a\"b\\c\u000ad""#);
    }

    #[test]
    fn snapshot_json_shape() {
        let snap = Snapshot {
            npu_percent: Some(37.25),
            procs: vec![Proc {
                pid: 4242,
                name: "python3".into(),
                project: Some(Project::VinoWhisper),
                cpu_percent: 12.5,
                ram_bytes: 812 * 1024 * 1024,
                ram_is_pss: true,
                npu_open: true,
            }],
            mem: Some((8192 * 1024 * 1024, 2048 * 1024 * 1024)),
            low_memory: false,
        };
        assert_eq!(
            snap.to_json(),
            r#"{"npu_percent":37.2,"ram_openvino_mib":812,"ram_total_mib":8192,"ram_available_mib":2048,"low_memory":false,"processes":[{"pid":4242,"name":"python3","project":"vinoWhisper","cpu_percent":12.5,"ram_mib":812,"ram_kind":"pss","npu_open":true}]}"#
        );
        assert_eq!(
            Snapshot::default().to_json(),
            r#"{"npu_percent":null,"ram_openvino_mib":0,"ram_total_mib":null,"ram_available_mib":null,"low_memory":false,"processes":[]}"#
        );
    }

    #[test]
    fn labels() {
        let p = Proc {
            pid: 1,
            name: "x".into(),
            project: None,
            cpu_percent: 0.0,
            ram_bytes: 0,
            ram_is_pss: false,
            npu_open: false,
        };
        assert_eq!(p.label(), "x pid 1: CPU 0.0%, RAM 0 MiB");
        assert_eq!(Snapshot::default().npu_label(), "NPU unavailable");
    }

    /// The test binary itself maps no OpenVINO, so a fake /proc entry is the
    /// only way to see a hit without OpenVINO installed: point the sampler at
    /// a proc root that maps it for our own pid.
    #[test]
    fn finds_a_process_by_its_mapped_library() {
        let root = std::env::temp_dir().join(format!("vinowatch-sample-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pid = std::process::id();
        // Every thread has its own /proc entry that reads the same, and
        // sysinfo lists each as a process: they must not be listed again.
        let tids = std::fs::read_dir("/proc/self/task").unwrap().flatten();
        for tid in tids {
            let dir = root.join(tid.file_name());
            std::fs::create_dir_all(dir.join("fd")).unwrap();
            std::fs::write(dir.join("maps"), "a-b r-xp 0 fd:01 1 /x/libopenvino.so\n").unwrap();
        }
        let mut sampler = Sampler::with_roots(root.clone(), root.join("no-accel"));
        let snap = sampler.poll();
        let me = snap
            .procs
            .iter()
            .find(|p| p.pid == pid)
            .expect("own pid listed");
        assert!(!me.npu_open);
        assert_eq!(snap.procs.len(), 1);
        assert_eq!(snap.npu_percent, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ram_label_and_low_memory() {
        let mut snap = Snapshot {
            procs: vec![Proc {
                pid: 1,
                name: "x".into(),
                project: None,
                cpu_percent: 0.0,
                ram_bytes: 1536 * 1024 * 1024,
                ram_is_pss: true,
                npu_open: false,
            }],
            mem: Some((8 * 1024 * 1024 * 1024, 4 * 1024 * 1024 * 1024)),
            ..Default::default()
        };
        assert_eq!(
            snap.ram_label(),
            "OpenVINO apps hold 1.5 GiB of 8.0 GiB RAM, 4.0 GiB available"
        );
        snap.low_memory = true;
        assert!(snap.ram_label().ends_with("(low)"));
        snap.mem = None;
        assert_eq!(snap.ram_label(), "OpenVINO apps hold 1.5 GiB RAM");
    }

    /// Writes a fake proc root with only a meminfo.
    fn low_memory_for(available_kib: u64, warn_below: u8) -> bool {
        let root = std::env::temp_dir().join(format!(
            "vinowatch-low-{}-{available_kib}-{warn_below}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("meminfo"),
            format!("MemTotal: 1000 kB\nMemAvailable: {available_kib} kB\n"),
        )
        .unwrap();
        let mut sampler = Sampler::with_roots(root.clone(), root.join("no-accel"));
        sampler.warn_below_percent = warn_below;
        let low = sampler.poll().low_memory;
        std::fs::remove_dir_all(&root).unwrap();
        low
    }

    #[test]
    fn low_memory_threshold() {
        assert!(low_memory_for(149, 15));
        assert!(!low_memory_for(150, 15)); // exactly at the threshold is fine
        assert!(!low_memory_for(0, 0)); // 0 turns the warning off
    }
}
