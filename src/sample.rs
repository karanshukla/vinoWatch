//! One poll of the machine: which processes use OpenVINO, and the NPU load.

use std::path::{Path, PathBuf};
use std::time::Instant;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::detect::{self, Project};
use crate::driver;
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
    /// `(total, available)` bytes: the machine's RAM, or the tightest cgroup
    /// limit around a listed process when that has less room left.
    pub mem: Option<(u64, u64)>,
    /// `mem` is a cgroup limit (a container, or a unit with `MemoryMax`), not
    /// the whole machine.
    pub cgroup_limited: bool,
    /// Available RAM is under the `--warn-below` share of the total.
    pub low_memory: bool,
    /// An `ivpu` device exposes its busy counter, even on the first poll.
    pub npu_present: bool,
    /// The venv interpreters of the Python OpenVINO apps listed this poll,
    /// for the stuck-driver probe (see `driver::python_of`).
    pub pythons: Vec<PathBuf>,
}

pub struct Sampler {
    sys: System,
    meter: Meter,
    proc_root: PathBuf,
    accel_root: PathBuf,
    cgroup_root: PathBuf,
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
            cgroup_root: "/sys/fs/cgroup".into(),
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
        let mut pythons = Vec::new();

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
                let listed = by_name
                    || match detect::uses_openvino(&self.proc_root, pid) {
                        Some(mapped) => mapped,
                        // Can't look inside: trust the executable's own name,
                        // not its arguments, so `sudo vinoauthface enroll`
                        // lists the child and not sudo.
                        None => detect::classify(&cmdline[..1], None).is_some(),
                    };
                if !listed {
                    return None;
                }
                pythons.extend(driver::python_of(&cmdline));
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

        let busy = npu::busy_us(&self.accel_root);
        let npu_present = busy.is_some();
        let npu_percent = self.meter.update(busy, Instant::now());
        pythons.sort();
        pythons.dedup();
        // The machine's RAM is one candidate; every cgroup limit above a
        // listed process is another. Whichever has the smallest share left
        // is what will run out first.
        let mut mem = detect::meminfo(&self.proc_root);
        let mut cgroup_limited = false;
        for p in &procs {
            let Some(cg) = detect::cgroup_headroom(&self.proc_root, &self.cgroup_root, p.pid)
            else {
                continue;
            };
            let share = |(t, a): (u64, u64)| a as f64 / t.max(1) as f64;
            if mem.is_none_or(|m| share(cg) < share(m)) {
                mem = Some(cg);
                cgroup_limited = true;
            }
        }
        let low_memory = mem.is_some_and(|(total, available)| {
            available.saturating_mul(100) < total.saturating_mul(self.warn_below_percent.into())
        });
        Snapshot {
            npu_percent,
            procs,
            mem,
            cgroup_limited,
            low_memory,
            npu_present,
            pythons,
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
            "{{\"npu_percent\":{npu},\"ram_openvino_mib\":{},\"ram_total_mib\":{total},\"ram_available_mib\":{available},\"ram_limit\":\"{}\",\"low_memory\":{},\"processes\":[{}]}}",
            mib(self.openvino_bytes()),
            if self.cgroup_limited {
                "cgroup"
            } else {
                "host"
            },
            self.low_memory,
            procs.join(",")
        )
    }

    /// What the listed processes hold between them. Exact with PSS; with the
    /// resident-size fallback it can count shared pages more than once.
    pub fn openvino_bytes(&self) -> u64 {
        self.procs.iter().map(|p| p.ram_bytes).sum()
    }

    /// `OpenVINO apps hold 2.1 GiB of 7.6 GiB RAM, 4.9 GiB available`; inside a
    /// limit, `... of 1.0 GiB cgroup limit, ...`.
    pub fn ram_label(&self) -> String {
        let gib = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);
        let used = gib(self.openvino_bytes());
        match self.mem {
            Some((total, available)) => format!(
                "OpenVINO apps hold {used:.1} GiB of {:.1} GiB {}, {:.1} GiB available{}",
                gib(total),
                if self.cgroup_limited {
                    "cgroup limit"
                } else {
                    "RAM"
                },
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
            cgroup_limited: false,
            low_memory: false,
            ..Default::default()
        };
        assert_eq!(
            snap.to_json(),
            r#"{"npu_percent":37.2,"ram_openvino_mib":812,"ram_total_mib":8192,"ram_available_mib":2048,"ram_limit":"host","low_memory":false,"processes":[{"pid":4242,"name":"python3","project":"vinoWhisper","cpu_percent":12.5,"ram_mib":812,"ram_kind":"pss","npu_open":true}]}"#
        );
        assert_eq!(
            Snapshot::default().to_json(),
            r#"{"npu_percent":null,"ram_openvino_mib":0,"ram_total_mib":null,"ram_available_mib":null,"ram_limit":"host","low_memory":false,"processes":[]}"#
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
        assert!(!snap.npu_present);
        // The test binary is no Python, so it offers no probe interpreter.
        assert!(snap.pythons.is_empty());
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
        snap.cgroup_limited = true;
        assert!(
            snap.ram_label()
                .contains("8.0 GiB cgroup limit, 4.0 GiB available")
        );
        snap.cgroup_limited = false;
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

    /// A listed process in a 1000 MiB cgroup with 100 MiB left must flag low
    /// memory even though the machine has plenty.
    #[test]
    fn a_cgroup_limit_can_be_the_binding_one() {
        let root = std::env::temp_dir().join(format!("vinowatch-cglow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pid = std::process::id();
        let (proc_root, cg) = (root.join("proc"), root.join("cg"));
        let mib = |n: u64| (n * 1024 * 1024).to_string();
        std::fs::create_dir_all(proc_root.join(format!("{pid}/fd"))).unwrap();
        std::fs::write(
            proc_root.join(format!("{pid}/maps")),
            "a-b r-xp 0 fd:01 1 /x/libopenvino.so\n",
        )
        .unwrap();
        std::fs::write(proc_root.join(format!("{pid}/cgroup")), "0::/box\n").unwrap();
        std::fs::write(
            proc_root.join("meminfo"),
            "MemTotal: 8000000 kB\nMemAvailable: 6000000 kB\n",
        )
        .unwrap();
        std::fs::create_dir_all(cg.join("box")).unwrap();
        std::fs::write(cg.join("box/memory.max"), mib(1000)).unwrap();
        std::fs::write(cg.join("box/memory.current"), mib(900)).unwrap();

        let mut sampler = Sampler::with_roots(proc_root.clone(), root.join("no-accel"));
        sampler.cgroup_root = cg.clone();
        sampler.warn_below_percent = 15;
        let snap = sampler.poll();
        assert!(snap.cgroup_limited);
        assert!(snap.low_memory);
        assert_eq!(snap.mem, Some((1000 << 20, 100 << 20)));

        // Plenty of room in the cgroup: the machine's figures stand.
        std::fs::write(cg.join("box/memory.current"), mib(100)).unwrap();
        let snap = sampler.poll();
        assert!(!snap.cgroup_limited);
        assert_eq!(snap.mem, Some((8_000_000 << 10, 6_000_000 << 10)));
        assert!(!snap.low_memory);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
