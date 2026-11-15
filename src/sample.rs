//! One poll of the machine: which processes use OpenVINO, and the NPU load.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::detect::{self, Memory, Project};
use crate::driver;
use crate::npu::{self, Meter};

const MIB: u64 = 1024 * 1024;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

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
    /// Maps OpenVINO or has it in its name. False for a vino app's helper
    /// listed by program name alone (its tray, say); true when its maps can't
    /// be read, since then we can't tell.
    pub openvino: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// System-wide; the driver has no per-process counter. `None` when there
    /// is no `ivpu` device or this is the first poll.
    pub npu_percent: Option<f64>,
    pub procs: Vec<Proc>,
    /// The machine's RAM, or the tightest cgroup limit around a listed
    /// process when that has less room left.
    pub memory: Option<Memory>,
    /// `memory` is a cgroup limit (a container, or a unit with `MemoryMax`), not
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
    /// Ours, to tell which processes' interpreters we may run.
    uid: Option<u32>,
    maps: MapsCache,
}

/// What each process's maps said, so idle processes aren't re-read every poll
/// (docs/how-it-works.md, "The poll's own cost"). Keyed by start time, so a
/// reused pid is a new process, and re-read when the virtual size changes,
/// which loading a library does.
#[derive(Default)]
struct MapsCache {
    seen: HashMap<u32, MapsReading>,
}

struct MapsReading {
    start_time: u64,
    virtual_size: u64,
    uses_openvino: Option<bool>,
}

impl MapsCache {
    fn uses_openvino(
        &mut self,
        pid: u32,
        start_time: u64,
        virtual_size: u64,
        read: impl FnOnce() -> Option<bool>,
    ) -> Option<bool> {
        match self.seen.get(&pid) {
            Some(known) if known.start_time == start_time && known.virtual_size == virtual_size => {
                known.uses_openvino
            }
            _ => {
                let uses_openvino = read();
                let reading = MapsReading {
                    start_time,
                    virtual_size,
                    uses_openvino,
                };
                self.seen.insert(pid, reading);
                uses_openvino
            }
        }
    }

    /// Forgets the processes that are gone.
    fn retain(&mut self, live: impl Fn(u32) -> bool) {
        self.seen.retain(|&pid, _| live(pid));
    }
}

/// The parts of a `Sampler` that looking at one process needs, apart from
/// sysinfo's list, which is borrowed while they are used.
struct Inspector<'a> {
    proc_root: &'a Path,
    uid: Option<u32>,
    maps: &'a mut MapsCache,
}

impl Inspector<'_> {
    /// The row for one sysinfo entry, if it belongs in the list, and the
    /// interpreter it offers the driver probe, if any.
    fn inspect(&mut self, pid: u32, process: &Process) -> Option<(Proc, Option<PathBuf>)> {
        let cmdline = command_line(process);
        if !is_userspace_process(process, &cmdline) {
            return None;
        }
        let name = process.name().to_string_lossy().into_owned();
        let named_openvino = name.to_lowercase().contains("openvino");
        let proc_root = self.proc_root;
        let maps_openvino =
            self.maps
                .uses_openvino(pid, process.start_time(), process.virtual_memory(), || {
                    detect::uses_openvino(proc_root, pid)
                });
        if !(named_openvino || maps_openvino == Some(true) || self.is_vino_program(pid, &cmdline)) {
            return None;
        }
        let python = self
            .runs_as_us(pid)
            .then(|| driver::python_of(&cmdline))
            .flatten();
        let cwd = process.cwd().map(Path::to_string_lossy);
        let pss = detect::pss_bytes(self.proc_root, pid);
        let listed = Proc {
            pid,
            ram_bytes: pss.unwrap_or_else(|| process.memory()),
            ram_is_pss: pss.is_some(),
            project: detect::classify(&cmdline, cwd.as_deref()),
            name,
            cpu_percent: process.cpu_usage(),
            npu_open: detect::holds_npu(self.proc_root, pid),
            // Unreadable maps (`None`) count as OpenVINO: we can't tell.
            openvino: named_openvino || maps_openvino != Some(false),
        };
        Some((listed, python))
    }

    /// A vino app's own program, listed even with no OpenVINO mapped. Only
    /// for pids the proc root has, so a fake root lists nothing from the
    /// real machine.
    fn is_vino_program(&self, pid: u32, cmdline: &[String]) -> bool {
        detect::program_project(cmdline).is_some() && self.proc_root.join(pid.to_string()).exists()
    }

    /// Security: argv[0] is the process owner's choice, and the probe runs
    /// it, so only our own processes may offer an interpreter.
    fn runs_as_us(&self, pid: u32) -> bool {
        detect::uids(self.proc_root, pid)
            .is_some_and(|(real, effective)| Some(real) == self.uid && Some(effective) == self.uid)
    }
}

fn command_line(process: &Process) -> Vec<String> {
    process
        .cmd()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// Not one of the threads sysinfo lists as processes of their own, and not a
/// kernel thread, which has no command line and no libraries.
fn is_userspace_process(process: &Process, cmdline: &[String]) -> bool {
    process.thread_kind().is_none() && !cmdline.is_empty()
}

/// Vino apps first, in `Project`'s order, then the busiest.
fn listing_order(a: &Proc, b: &Proc) -> Ordering {
    a.project
        .is_none()
        .cmp(&b.project.is_none())
        .then(a.project.cmp(&b.project))
        .then(b.cpu_percent.total_cmp(&a.cpu_percent))
        .then(a.pid.cmp(&b.pid))
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
            // /proc/<pid> belongs to the process's effective uid.
            uid: std::fs::metadata("/proc/self").ok().map(|m| m.uid()),
            maps: MapsCache::default(),
        }
    }

    /// CPU % is a delta, so the first poll reads 0 for everything. Call this
    /// once, wait, then poll for real.
    pub fn poll(&mut self) -> Snapshot {
        self.refresh_processes();
        let (mut procs, mut pythons) = self.list_processes();
        procs.sort_by(listing_order);
        pythons.sort();
        pythons.dedup();

        let busy = npu::busy_us(&self.accel_root);
        let npu_percent = self.meter.update(busy, Instant::now());
        let (memory, cgroup_limited) = self.binding_memory(&procs);
        Snapshot {
            npu_percent,
            npu_present: busy.is_some(),
            low_memory: memory.is_some_and(|m| m.below_percent(self.warn_below_percent)),
            procs,
            memory,
            cgroup_limited,
            pythons,
        }
    }

    fn refresh_processes(&mut self) {
        // Without tasks: sysinfo would read every thread, only for
        // `is_userspace_process` to drop them.
        let kind = ProcessRefreshKind::nothing()
            .without_tasks()
            .with_cpu()
            .with_memory()
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet);
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    }

    /// The rows, unsorted, and the interpreters they offer the driver probe.
    fn list_processes(&mut self) -> (Vec<Proc>, Vec<PathBuf>) {
        let processes = self.sys.processes();
        let mut inspector = Inspector {
            proc_root: &self.proc_root,
            uid: self.uid,
            maps: &mut self.maps,
        };
        let (procs, pythons): (Vec<Proc>, Vec<Option<PathBuf>>) = processes
            .iter()
            .filter_map(|(pid, process)| inspector.inspect(pid.as_u32(), process))
            .unzip();
        self.maps
            .retain(|pid| processes.contains_key(&Pid::from_u32(pid)));
        (procs, pythons.into_iter().flatten().collect())
    }

    /// What will run out first: the machine's RAM, or a cgroup limit above a
    /// listed process with a smaller share left. The flag says it is a limit.
    fn binding_memory(&self, procs: &[Proc]) -> (Option<Memory>, bool) {
        let mut binding = detect::meminfo(&self.proc_root);
        let mut cgroup_limited = false;
        for listed in procs {
            let limit = detect::cgroup_headroom(&self.proc_root, &self.cgroup_root, listed.pid);
            if let Some(limit) = limit
                && binding.is_none_or(|current| limit.tighter_than(current))
            {
                binding = Some(limit);
                cgroup_limited = true;
            }
        }
        (binding, cgroup_limited)
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
                    "{{\"pid\":{},\"name\":{},\"project\":{},\"cpu_percent\":{:.1},\"ram_mib\":{},\"ram_kind\":\"{}\",\"npu_open\":{},\"openvino\":{}}}",
                    p.pid,
                    json_string(&p.name),
                    project,
                    p.cpu_percent,
                    p.ram_bytes / MIB,
                    if p.ram_is_pss { "pss" } else { "rss" },
                    p.npu_open,
                    p.openvino,
                )
            })
            .collect();
        let mib = |bytes: u64| bytes / MIB;
        let (total, available) = match self.memory {
            Some(memory) => (
                mib(memory.total).to_string(),
                mib(memory.available).to_string(),
            ),
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
        let gib = |bytes: u64| bytes as f64 / GIB;
        let used = gib(self.openvino_bytes());
        match self.memory {
            Some(Memory { total, available }) => format!(
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
        let helper = if self.openvino { "" } else { ", helper" };
        format!(
            "{who} pid {}: CPU {:.1}%, RAM {} MiB{npu}{helper}",
            self.pid,
            self.cpu_percent,
            self.ram_bytes / MIB,
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
pub mod testing {
    use super::Proc;

    /// An OpenVINO process that is idle and holds nothing. Tests override
    /// the fields they are about.
    pub fn proc_row(pid: u32) -> Proc {
        Proc {
            pid,
            name: "x".into(),
            project: None,
            cpu_percent: 0.0,
            ram_bytes: 0,
            ram_is_pss: true,
            npu_open: false,
            openvino: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::proc_row;
    use super::*;
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::Duration;

    const OPENVINO_MAPS: &str = "a-b r-xp 0 fd:01 1 /x/libopenvino.so\n";

    /// A fake `/proc/<pid>` whose maps list an OpenVINO library.
    fn fake_openvino_process(proc_root: &Path, pid: u32) {
        let dir = proc_root.join(pid.to_string());
        fs::create_dir_all(dir.join("fd")).unwrap();
        fs::write(dir.join("maps"), OPENVINO_MAPS).unwrap();
    }

    fn sampler_at(proc_root: &Path) -> Sampler {
        Sampler::with_roots(proc_root.to_path_buf(), proc_root.join("no-accel"))
    }

    /// A real process whose `argv[0]` is `program`, killed on drop. Real,
    /// because sysinfo lists from the machine's `/proc`, not the fake root.
    struct Running(Child);

    impl Running {
        fn as_program(program: &str) -> Self {
            let child = Command::new("/bin/sleep")
                .arg0(program)
                .arg("30")
                .spawn()
                .unwrap();
            // Until the exec, the child's command line is still ours.
            let cmdline = format!("/proc/{}/cmdline", child.id());
            let deadline = Instant::now() + Duration::from_secs(5);
            let expected = format!("{program}\0");
            while !fs::read(&cmdline).is_ok_and(|c| c.starts_with(expected.as_bytes())) {
                assert!(Instant::now() < deadline, "sleep never started");
                std::thread::sleep(Duration::from_millis(10));
            }
            Running(child)
        }

        fn pid(&self) -> u32 {
            self.0.id()
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn proc_with(project: Option<Project>, cpu_percent: f32, pid: u32) -> Proc {
        Proc {
            project,
            cpu_percent,
            ..proc_row(pid)
        }
    }

    #[test]
    fn vino_apps_are_listed_first_then_the_busiest() {
        let mut procs = [
            proc_with(None, 90.0, 1),
            proc_with(Some(Project::VinoSpeak), 5.0, 2),
            proc_with(None, 95.0, 3),
            proc_with(Some(Project::VinoWhisper), 0.0, 4),
            proc_with(Some(Project::VinoSpeak), 50.0, 5),
            proc_with(Some(Project::VinoSpeak), 50.0, 6),
        ];
        procs.sort_by(listing_order);
        let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
        assert_eq!(pids, [4, 5, 6, 2, 3, 1]);
    }

    /// vinoWhisper's tray never maps OpenVINO: it is listed for its program
    /// name, as a helper.
    #[test]
    fn a_vino_apps_own_program_is_listed_as_a_helper() {
        let helper = Running::as_program("/opt/bin/vinowhisper-tray");
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(helper.pid().to_string())).unwrap();

        let snap = sampler_at(root.path()).poll();
        let [listed] = snap.procs.as_slice() else {
            panic!("only the helper is under this proc root: {:?}", snap.procs);
        };
        assert_eq!(listed.pid, helper.pid());
        assert_eq!(listed.project, Some(Project::VinoWhisper));
        assert!(!listed.openvino);
        assert_eq!(listed.label().rsplit(", ").next(), Some("helper"));
    }

    /// The other side of the rule above: the program name alone lists nothing
    /// that the sampler's proc root doesn't have.
    #[test]
    fn a_vino_program_outside_the_proc_root_is_not_listed() {
        let _helper = Running::as_program("/opt/bin/vinowhisper-tray");
        let root = tempfile::tempdir().unwrap();
        assert_eq!(sampler_at(root.path()).poll().procs, []);
    }

    #[test]
    fn ram_is_pss_where_smaps_rollup_reads() {
        let root = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        fake_openvino_process(root.path(), pid);
        fs::write(
            root.path().join(format!("{pid}/smaps_rollup")),
            "Rss: 900 kB\nPss: 512 kB\n",
        )
        .unwrap();
        let snap = sampler_at(root.path()).poll();
        assert!(snap.procs[0].ram_is_pss);
        assert_eq!(snap.procs[0].ram_bytes, 512 * 1024);
        assert!(snap.to_json().contains(r#""ram_kind":"pss""#));
    }

    #[test]
    fn ram_falls_back_to_resident_size_without_smaps_rollup() {
        let root = tempfile::tempdir().unwrap();
        fake_openvino_process(root.path(), std::process::id());
        let snap = sampler_at(root.path()).poll();
        assert!(!snap.procs[0].ram_is_pss);
        assert!(snap.procs[0].ram_bytes > 0, "sysinfo's resident size");
        assert!(snap.to_json().contains(r#""ram_kind":"rss""#));
    }

    #[test]
    fn maps_are_read_again_only_for_a_new_or_resized_process() {
        let mut cache = MapsCache::default();
        let mut reads = 0;
        let mut check = |cache: &mut MapsCache, start, vsize, maps| {
            cache.uses_openvino(7, start, vsize, || {
                reads += 1;
                maps
            })
        };
        assert_eq!(check(&mut cache, 100, 4096, Some(false)), Some(false));
        // Unchanged: the first answer stands, even if the maps now differ.
        assert_eq!(check(&mut cache, 100, 4096, Some(true)), Some(false));
        // It mapped something new.
        assert_eq!(check(&mut cache, 100, 8192, Some(true)), Some(true));
        // Same pid, another process.
        assert_eq!(check(&mut cache, 200, 8192, None), None);
        assert_eq!(reads, 3);
        cache.retain(|_| false);
        assert!(cache.seen.is_empty());
    }

    #[test]
    fn json_escapes_what_a_process_name_can_contain() {
        assert_eq!(json_string("a\"b\\c\nd"), r#""a\"b\\c\u000ad""#);
    }

    #[test]
    fn snapshot_json_shape() {
        let snap = Snapshot {
            npu_percent: Some(37.25),
            procs: vec![Proc {
                name: "python3".into(),
                project: Some(Project::VinoWhisper),
                cpu_percent: 12.5,
                ram_bytes: 812 * 1024 * 1024,
                npu_open: true,
                ..proc_row(4242)
            }],
            memory: Some(Memory {
                total: 8192 * MIB,
                available: 2048 * MIB,
            }),
            cgroup_limited: false,
            low_memory: false,
            ..Default::default()
        };
        assert_eq!(
            snap.to_json(),
            r#"{"npu_percent":37.2,"ram_openvino_mib":812,"ram_total_mib":8192,"ram_available_mib":2048,"ram_limit":"host","low_memory":false,"processes":[{"pid":4242,"name":"python3","project":"vinoWhisper","cpu_percent":12.5,"ram_mib":812,"ram_kind":"pss","npu_open":true,"openvino":true}]}"#
        );
        assert_eq!(
            Snapshot::default().to_json(),
            r#"{"npu_percent":null,"ram_openvino_mib":0,"ram_total_mib":null,"ram_available_mib":null,"ram_limit":"host","low_memory":false,"processes":[]}"#
        );
    }

    /// Whatever writes the JSON must keep these bytes: scripts read them.
    #[test]
    fn json_numbers_have_one_decimal_and_sizes_are_whole_mebibytes() {
        let snap = Snapshot {
            npu_percent: Some(100.0),
            procs: vec![
                Proc {
                    cpu_percent: 0.05,
                    ram_bytes: MIB - 1,
                    ram_is_pss: false,
                    openvino: false,
                    ..proc_row(1)
                },
                Proc {
                    cpu_percent: 399.96,
                    ram_bytes: 3 * MIB + 1,
                    ..proc_row(2)
                },
            ],
            memory: Some(Memory {
                total: 1000 * MIB,
                available: 100 * MIB + 5,
            }),
            cgroup_limited: true,
            low_memory: true,
            ..Default::default()
        };
        assert_eq!(
            snap.to_json(),
            concat!(
                r#"{"npu_percent":100.0,"ram_openvino_mib":4,"ram_total_mib":1000,"ram_available_mib":100,"ram_limit":"cgroup","low_memory":true,"processes":["#,
                r#"{"pid":1,"name":"x","project":null,"cpu_percent":0.1,"ram_mib":0,"ram_kind":"rss","npu_open":false,"openvino":false},"#,
                r#"{"pid":2,"name":"x","project":null,"cpu_percent":400.0,"ram_mib":3,"ram_kind":"pss","npu_open":false,"openvino":true}]}"#
            )
        );
        let idle = Snapshot {
            npu_percent: Some(0.0),
            ..Default::default()
        };
        assert!(idle.to_json().starts_with(r#"{"npu_percent":0.0,"#));
    }

    #[test]
    fn json_names_keep_their_escapes() {
        let named = |name: &str| {
            let snap = Snapshot {
                procs: vec![Proc {
                    name: name.into(),
                    ..proc_row(1)
                }],
                ..Default::default()
            };
            let json = snap.to_json();
            let start = json.find(r#""name":"#).unwrap() + 7;
            let end = json.find(r#","project""#).unwrap();
            json[start..end].to_string()
        };
        assert_eq!(named("a\"b\\c\nd"), r#""a\"b\\c\u000ad""#);
        assert_eq!(named("tab\there"), r#""tab\u0009here""#);
        assert_eq!(
            named("\u{1}\u{1f}\r\u{8}\u{c}"),
            r#""\u0001\u001f\u000d\u0008\u000c""#
        );
        // Not escaped: a slash, DEL, and anything outside ASCII.
        assert_eq!(named("a/b\u{7f}é\u{2028}🙂"), "\"a/b\u{7f}é\u{2028}🙂\"");
    }

    #[test]
    fn labels() {
        let p = Proc {
            ram_is_pss: false,
            ..proc_row(1)
        };
        assert_eq!(p.label(), "x pid 1: CPU 0.0%, RAM 0 MiB");
        let helper = Proc {
            openvino: false,
            ..p
        };
        assert_eq!(helper.label(), "x pid 1: CPU 0.0%, RAM 0 MiB, helper");
        assert_eq!(Snapshot::default().npu_label(), "NPU unavailable");
    }

    /// The test binary itself maps no OpenVINO, so a fake /proc entry is the
    /// only way to see a hit without OpenVINO installed: point the sampler at
    /// a proc root that maps it for our own pid.
    #[test]
    fn finds_a_process_by_its_mapped_library() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let pid = std::process::id();
        // Every thread has its own /proc entry that reads the same, and
        // sysinfo lists each as a process: they must not be listed again.
        let tids = fs::read_dir("/proc/self/task").unwrap().flatten();
        for tid in tids {
            let tid = tid.file_name().to_string_lossy().parse().unwrap();
            fake_openvino_process(root, tid);
        }
        let snap = sampler_at(root).poll();
        let me = snap
            .procs
            .iter()
            .find(|p| p.pid == pid)
            .expect("own pid listed");
        assert!(!me.npu_open);
        assert!(me.openvino);
        assert_eq!(snap.procs.len(), 1);
        assert_eq!(snap.npu_percent, None);
        assert!(!snap.npu_present);
        // The test binary is no Python, so it offers no probe interpreter.
        assert!(snap.pythons.is_empty());
    }

    #[test]
    fn ram_label_and_low_memory() {
        let mut snap = Snapshot {
            procs: vec![Proc {
                ram_bytes: 1536 * 1024 * 1024,
                ..proc_row(1)
            }],
            memory: Some(Memory {
                total: 8192 * MIB,
                available: 4096 * MIB,
            }),
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
        snap.memory = None;
        assert_eq!(snap.ram_label(), "OpenVINO apps hold 1.5 GiB RAM");
    }

    /// Writes a fake proc root with only a meminfo.
    fn low_memory_for(available_kib: u64, warn_below: u8) -> bool {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("meminfo"),
            format!("MemTotal: 1000 kB\nMemAvailable: {available_kib} kB\n"),
        )
        .unwrap();
        let mut sampler = sampler_at(root.path());
        sampler.warn_below_percent = warn_below;
        sampler.poll().low_memory
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
        let scratch = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        let (proc_root, cg) = (scratch.path().join("proc"), scratch.path().join("cg"));
        let mib = |n: u64| (n * MIB).to_string();
        fake_openvino_process(&proc_root, pid);
        fs::write(proc_root.join(format!("{pid}/cgroup")), "0::/box\n").unwrap();
        fs::write(
            proc_root.join("meminfo"),
            "MemTotal: 8000000 kB\nMemAvailable: 6000000 kB\n",
        )
        .unwrap();
        fs::create_dir_all(cg.join("box")).unwrap();
        fs::write(cg.join("box/memory.max"), mib(1000)).unwrap();
        fs::write(cg.join("box/memory.current"), mib(900)).unwrap();

        let mut sampler = sampler_at(&proc_root);
        sampler.cgroup_root = cg.clone();
        sampler.warn_below_percent = 15;
        let snap = sampler.poll();
        assert!(snap.cgroup_limited);
        assert!(snap.low_memory);
        assert_eq!(
            snap.memory,
            Some(Memory {
                total: 1000 * MIB,
                available: 100 * MIB
            })
        );

        // Plenty of room in the cgroup: the machine's figures stand.
        fs::write(cg.join("box/memory.current"), mib(100)).unwrap();
        let snap = sampler.poll();
        assert!(!snap.cgroup_limited);
        assert_eq!(
            snap.memory,
            Some(Memory {
                total: 8_000_000 << 10,
                available: 6_000_000 << 10
            })
        );
        assert!(!snap.low_memory);
    }

    /// Another user can start anything with `argv[0]` set to a path named
    /// `python3`; the probe would then run their program as us.
    #[test]
    fn only_our_own_processes_offer_an_interpreter() {
        let fake_python = PathBuf::from("/tmp/vinowhisper/python3");
        let app = Running::as_program("/tmp/vinowhisper/python3");
        let pid = app.pid();
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        fake_openvino_process(root, pid);
        let me = sampler_at(root).uid.unwrap();
        let other = me.wrapping_add(1);
        for (real, effective, offered) in [
            (other, other, false),
            (me, other, false),
            (other, me, false),
            (me, me, true),
        ] {
            fs::write(
                root.join(format!("{pid}/status")),
                format!("Name:\tsleep\nUid:\t{real}\t{effective}\t{effective}\t{effective}\n"),
            )
            .unwrap();
            let snap = sampler_at(root).poll();
            assert!(snap.procs.iter().any(|p| p.pid == pid), "listed either way");
            assert_eq!(
                snap.pythons.contains(&fake_python),
                offered,
                "uid {real}/{effective}"
            );
        }
        // No status to read: not offered.
        fs::remove_file(root.join(format!("{pid}/status"))).unwrap();
        assert!(sampler_at(root).poll().pythons.is_empty());
    }
}
