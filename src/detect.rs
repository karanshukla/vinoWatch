//! Deciding which processes are OpenVINO users, and which vino app they belong to.
//!
//! Everything takes the `/proc` root as a parameter so the tests can build a
//! fake one.

use std::fmt;
use std::fs;
use std::path::Path;

/// The vino apps that run models through OpenVINO.
#[allow(
    clippy::enum_variant_names,
    reason = "the prefix is the apps' own name, and Display prints it back"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Project {
    VinoWhisper,
    VinoSpeak,
    VinoPop,
    VinoAuthFace,
    VinoSearch,
}

impl fmt::Display for Project {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::VinoWhisper => "vinoWhisper",
            Self::VinoSpeak => "vinoSpeak",
            Self::VinoPop => "vinoPop",
            Self::VinoAuthFace => "vinoAuthFace",
            Self::VinoSearch => "vinoSearch",
        })
    }
}

/// Which project a process belongs to, from its command line and working
/// directory, which alone names a `python3 main.py` vinoPop
/// (pinned by `tests::vinopop_is_found_by_its_checkout_directory`).
pub fn classify(cmdline: &[String], cwd: Option<&str>) -> Option<Project> {
    let mut haystack = cmdline.join(" ").to_lowercase();
    if let Some(cwd) = cwd {
        haystack.push(' ');
        haystack.push_str(&cwd.to_lowercase());
    }
    const MARKERS: [(&str, Project); 7] = [
        ("vinowhisper", Project::VinoWhisper),
        ("vinospeak", Project::VinoSpeak),
        ("vino_speak", Project::VinoSpeak),
        ("vinoauthface", Project::VinoAuthFace),
        ("face-auth", Project::VinoAuthFace),
        ("vinosearch", Project::VinoSearch),
        ("vinopop", Project::VinoPop),
    ];
    MARKERS
        .iter()
        .find(|(marker, _)| haystack.contains(marker))
        .map(|&(_, project)| project)
}

/// Which project the program itself is, from file names only: argv[0], or for
/// an interpreter its script or `-m` module, so a helper that never loads
/// OpenVINO counts and `pytest` in a vino checkout doesn't
/// (pinned by `tests::paths_and_arguments_are_not_the_program`).
pub fn program_project(cmdline: &[String]) -> Option<Project> {
    let file_name = |arg: &String| arg.rsplit('/').next().unwrap_or_default().to_owned();
    let program = file_name(cmdline.first()?);
    let mut names = vec![program.clone()];
    if program.starts_with("python") {
        match cmdline.get(1).map(String::as_str) {
            Some("-m") => names.extend(cmdline.get(2).cloned()),
            Some(arg) if !arg.starts_with('-') => names.push(file_name(&arg.to_owned())),
            _ => {}
        }
    }
    names
        .iter()
        .find_map(|name| classify(std::slice::from_ref(name), None))
}

/// Whether a `/proc/<pid>/maps` listing maps an OpenVINO library.
///
/// Only the file name counts, so a data file that happens to have
/// `libopenvino` somewhere in its path doesn't.
pub fn maps_openvino(maps: &str) -> bool {
    maps.lines().any(|line| {
        let Some(path) = path_column(line) else {
            return false;
        };
        let path = path.trim().trim_end_matches(" (deleted)");
        path.rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("libopenvino"))
    })
}

fn path_column(line: &str) -> Option<&str> {
    const COLUMNS: usize = 6;
    line.splitn(COLUMNS, char::is_whitespace).nth(COLUMNS - 1)
}

/// Whether `pid` has an OpenVINO library mapped. `None` when we may not read
/// the maps (other users' and non-dumpable setuid processes like
/// `vinoauthface-auth`): "can't tell". A process that has gone is `Some(false)`
/// (pinned by `tests::a_process_that_is_gone_is_not_an_openvino_user`,
/// `tests::unreadable_maps_mean_cant_tell`).
pub fn uses_openvino(proc_root: &Path, pid: u32) -> Option<bool> {
    match fs::read_to_string(proc_root.join(format!("{pid}/maps"))) {
        Ok(maps) => Some(maps_openvino(&maps)),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => None,
        Err(_) => Some(false),
    }
}

/// Whether `pid` has an `/dev/accel/*` node open, i.e. it is talking to the NPU.
pub fn holds_npu(proc_root: &Path, pid: u32) -> bool {
    let Ok(fds) = fs::read_dir(proc_root.join(format!("{pid}/fd"))) else {
        return false;
    };
    fds.flatten()
        .any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target.starts_with("/dev/accel")))
}

/// The real and effective uid of `pid`, from the `Uid:` line of `status`.
pub fn uids(proc_root: &Path, pid: u32) -> Option<(u32, u32)> {
    let text = fs::read_to_string(proc_root.join(format!("{pid}/status"))).ok()?;
    let line = text.lines().find_map(|l| l.strip_prefix("Uid:"))?;
    let mut ids = line.split_whitespace().map(|n| n.parse().ok());
    Some((ids.next()??, ids.next()??))
}

/// Proportional set size in bytes: resident memory with each shared page
/// divided among the processes sharing it, so figures from several processes
/// can be added up. `None` when `smaps_rollup` can't be read.
pub fn pss_bytes(proc_root: &Path, pid: u32) -> Option<u64> {
    let text = fs::read_to_string(proc_root.join(format!("{pid}/smaps_rollup"))).ok()?;
    kib_field(&text, "Pss:").map(|kib| kib * 1024)
}

/// Memory against its ceiling: the machine's RAM, or a cgroup limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
}

impl Memory {
    /// The share of its own total still free, 0 to 1.
    pub fn share_left(self) -> f64 {
        self.available as f64 / self.total.max(1) as f64
    }

    /// Runs out before `other`: the smaller share left, not the fewer bytes
    /// (pinned by `tests::the_tighter_memory_is_the_one_with_the_smaller_share_left`).
    pub fn tighter_than(self, other: Memory) -> bool {
        self.share_left() < other.share_left()
    }

    /// Whether less than `percent` of the total is available; 0 never is
    /// (pinned by `tests::memory_is_low_under_the_warning_share_and_fine_at_it`,
    /// `tests::a_warning_share_of_zero_never_fires`).
    pub fn below_percent(self, percent: u8) -> bool {
        self.available.saturating_mul(100) < self.total.saturating_mul(percent.into())
    }
}

/// `MemTotal` and `MemAvailable` from `/proc/meminfo`.
pub fn meminfo(proc_root: &Path) -> Option<Memory> {
    let text = fs::read_to_string(proc_root.join("meminfo")).ok()?;
    Some(Memory {
        total: kib_field(&text, "MemTotal:")? * 1024,
        available: kib_field(&text, "MemAvailable:")? * 1024,
    })
}

/// The tightest memory limit above `pid` in the cgroup v2 tree, or `None` if
/// no cgroup around it sets one (or the tree isn't cgroup v2).
pub fn cgroup_headroom(proc_root: &Path, cgroup_root: &Path, pid: u32) -> Option<Memory> {
    let mut dir = cgroup_root.join(cgroup_v2_path(proc_root, pid)?);
    let mut tightest: Option<Memory> = None;
    loop {
        if let Some(limited) = cgroup_memory(&dir)
            && tightest.is_none_or(|t| limited.tighter_than(t))
        {
            tightest = Some(limited);
        }
        if dir == cgroup_root || !dir.pop() || !dir.starts_with(cgroup_root) {
            return tightest;
        }
    }
}

/// `pid`'s cgroup, relative to the cgroup root. v2 lists it on the single
/// `0::/path` line; v1 has one line per controller and none like it.
fn cgroup_v2_path(proc_root: &Path, pid: u32) -> Option<String> {
    let listing = fs::read_to_string(proc_root.join(format!("{pid}/cgroup"))).ok()?;
    let path = listing.lines().find_map(|l| l.strip_prefix("0::"))?;
    Some(path.trim_start_matches('/').to_owned())
}

/// One cgroup's limit and what is left under it, if it sets a limit.
///
/// "Used" is `memory.current` minus inactive file cache, like `docker stats`,
/// which gives way under pressure
/// (pinned by `tests::a_parents_limit_counts_inactive_cache_as_free`).
fn cgroup_memory(dir: &Path) -> Option<Memory> {
    let limit = read_u64(&dir.join("memory.max"))?;
    let current = read_u64(&dir.join("memory.current")).unwrap_or(0);
    let used = current.saturating_sub(inactive_file_bytes(dir));
    Some(Memory {
        total: limit,
        available: limit.saturating_sub(used),
    })
}

fn inactive_file_bytes(cgroup_dir: &Path) -> u64 {
    fs::read_to_string(cgroup_dir.join("memory.stat"))
        .ok()
        .and_then(|stat| {
            stat.lines()
                .find_map(|l| l.strip_prefix("inactive_file "))
                .and_then(|n| n.trim().parse().ok())
        })
        .unwrap_or(0)
}

/// A cgroup file holding one number; `max` (no limit) and anything else
/// unreadable count as absent.
fn read_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn kib_field(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn names_each_project_from_its_command_line() {
        let cases = [
            (
                &[
                    "/h/vinoWhisper/.venv/bin/python",
                    "/h/vinoWhisper/.venv/bin/vinowhisper-server",
                    "--device",
                    "auto",
                ][..],
                Project::VinoWhisper,
            ),
            (
                &["python3", "-m", "vinowhisper.caption"][..],
                Project::VinoWhisper,
            ),
            (
                &["/h/vinoSpeak/.venv/bin/vinospeak", "hello"][..],
                Project::VinoSpeak,
            ),
            (&["python", "-m", "vino_speak.cli"][..], Project::VinoSpeak),
            (
                &["/usr/local/bin/vinoauthface-auth"][..],
                Project::VinoAuthFace,
            ),
            (
                &["/usr/local/bin/vinoauthface", "enroll"][..],
                Project::VinoAuthFace,
            ),
            (&["/usr/local/bin/face-auth"][..], Project::VinoAuthFace),
            (
                &["/home/u/.cargo/bin/vinosearch", "krunner"][..],
                Project::VinoSearch,
            ),
        ];
        for (argv, want) in cases {
            assert_eq!(classify(&args(argv), None), Some(want), "{argv:?}");
        }
    }

    #[test]
    fn vinopop_is_found_by_its_launcher_or_checkout_path() {
        for argv in [
            &["/usr/bin/python3", "/home/u/vinopop/vinopop", "--no-tray"][..],
            &[
                "/home/u/src/vinoPop/.venv/bin/python",
                "/home/u/src/vinoPop/main.py",
            ],
        ] {
            assert_eq!(
                classify(&args(argv), Some("/home/u")),
                Some(Project::VinoPop),
                "{argv:?}"
            );
        }
    }

    #[test]
    fn vinopop_is_found_by_its_checkout_directory() {
        let argv = args(&["python3", "main.py", "--no-tray"]);
        assert_eq!(classify(&argv, None), None);
        assert_eq!(
            classify(&argv, Some("/home/dev/workspace/vinoPop")),
            Some(Project::VinoPop)
        );
    }

    #[test]
    fn a_projects_own_programs_are_named_by_file_name_only() {
        for (argv, project) in [
            (
                &["/home/k/.local/bin/vinowhisper-tray", "--hidden"][..],
                Some(Project::VinoWhisper),
            ),
            (
                &["/home/k/.local/bin/vinowhisper-gui"],
                Some(Project::VinoWhisper),
            ),
            (
                &[
                    "/home/k/Development/vinoWhisper/.venv/bin/python",
                    "/home/k/.local/bin/vinowhisper-dictate",
                    "--json",
                ],
                Some(Project::VinoWhisper),
            ),
            (
                &["python3", "-m", "vinowhisper.server"],
                Some(Project::VinoWhisper),
            ),
            (
                &["python3", "/home/k/Development/vinoPop/vinopop"],
                Some(Project::VinoPop),
            ),
        ] {
            assert_eq!(program_project(&args(argv)), project, "{argv:?}");
        }
    }

    #[test]
    fn paths_and_arguments_are_not_the_program() {
        for argv in [
            &["/home/k/Development/vinoWhisper/.venv/bin/pytest", "-q"][..],
            &["uv", "run", "vinowhisper-caption"],
            &["kwrite", "/home/k/notes/vinowhisper.txt"],
            &["sudo", "vinoauthface", "enroll"],
            &[
                "python3",
                "/home/k/Development/vinoWhisper/scripts/cleanup_eval.py",
            ],
        ] {
            assert_eq!(program_project(&args(argv)), None, "{argv:?}");
        }
    }

    #[test]
    fn unrelated_processes_have_no_project() {
        assert_eq!(
            classify(&args(&["python3", "train.py"]), Some("/home/dev")),
            None
        );
    }

    #[test]
    fn maps_match_on_the_file_name_only() {
        let real = "7f00-7f10 r-xp 00000000 fd:01 42 /opt/intel/openvino/runtime/lib/libopenvino.so.2025.3.0\n";
        let plugin = "7f00-7f10 r-xp 00000000 fd:01 43 /usr/lib/libopenvino_intel_npu_plugin.so\n";
        let deleted = "7f00-7f10 r-xp 00000000 fd:01 44 /tmp/libopenvino_c.so (deleted)\n";
        let data = "7f00-7f10 r--p 00000000 fd:01 45 /home/u/libopenvino-notes/model.bin\n";
        let spaced = "7f00-7f10 r--p 00000000 fd:01 46 /home/my user/lib/libopenvino.so\n";
        assert!(maps_openvino(real));
        assert!(maps_openvino(plugin));
        assert!(maps_openvino(deleted));
        assert!(maps_openvino(spaced));
        assert!(!maps_openvino(data));
        assert!(!maps_openvino("7f00-7f10 rw-p 00000000 00:00 0\n"));
        assert!(!maps_openvino(""));
    }

    #[test]
    fn reads_maps_and_fds_from_a_proc_root() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        fs::create_dir_all(root.join("7/fd")).unwrap();
        fs::write(
            root.join("7/maps"),
            "a-b r-xp 0 fd:01 1 /x/libopenvino.so\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("/dev/accel/accel0", root.join("7/fd/3")).unwrap();
        std::os::unix::fs::symlink("/dev/null", root.join("7/fd/4")).unwrap();
        fs::create_dir_all(root.join("8/fd")).unwrap();
        std::os::unix::fs::symlink("/dev/null", root.join("8/fd/3")).unwrap();

        assert_eq!(uses_openvino(root, 7), Some(true));
        assert!(holds_npu(root, 7));
        assert_eq!(uses_openvino(root, 8), Some(false));
        assert!(!holds_npu(root, 8));
    }

    #[test]
    fn a_process_that_is_gone_is_not_an_openvino_user() {
        let scratch = tempfile::tempdir().unwrap();
        assert_eq!(uses_openvino(scratch.path(), 9), Some(false));
        assert!(!holds_npu(scratch.path(), 9));
    }

    #[test]
    fn unreadable_maps_mean_cant_tell() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        fs::create_dir_all(root.join("10")).unwrap();
        fs::write(root.join("10/maps"), "").unwrap();
        fs::set_permissions(root.join("10/maps"), fs::Permissions::from_mode(0o000)).unwrap();
        let running_as_root = fs::read(root.join("10/maps")).is_ok();
        if !running_as_root {
            assert_eq!(uses_openvino(root, 10), None);
        }
    }

    struct Cgroups {
        _scratch: tempfile::TempDir,
        proc_root: std::path::PathBuf,
        cg: std::path::PathBuf,
    }

    fn mib(n: u64) -> u64 {
        n * 1024 * 1024
    }

    impl Cgroups {
        fn new() -> Self {
            let scratch = tempfile::tempdir().unwrap();
            let proc_root = scratch.path().join("proc");
            let cg = scratch.path().join("cg");
            Self {
                _scratch: scratch,
                proc_root,
                cg,
            }
        }

        fn write(&self, dir: &str, max: &str, current: u64, inactive: u64) {
            let d = self.cg.join(dir);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("memory.max"), format!("{max}\n")).unwrap();
            fs::write(d.join("memory.current"), mib(current).to_string()).unwrap();
            fs::write(
                d.join("memory.stat"),
                format!("anon 1\ninactive_file {}\n", mib(inactive)),
            )
            .unwrap();
        }

        fn put_pid_in(&self, pid: u32, cgroup_line: &str) {
            fs::create_dir_all(self.proc_root.join(pid.to_string())).unwrap();
            fs::write(
                self.proc_root.join(format!("{pid}/cgroup")),
                format!("{cgroup_line}\n"),
            )
            .unwrap();
        }

        fn headroom(&self, pid: u32) -> Option<Memory> {
            cgroup_headroom(&self.proc_root, &self.cg, pid)
        }
    }

    #[test]
    fn a_parents_limit_counts_inactive_cache_as_free() {
        let tree = Cgroups::new();
        tree.write("a/b", "max", 100, 0);
        tree.write("a", &mib(1000).to_string(), 900, 300);
        tree.put_pid_in(7, "0::/a/b");
        assert_eq!(
            tree.headroom(7),
            Some(Memory {
                total: mib(1000),
                available: mib(400)
            })
        );
    }

    #[test]
    fn a_tighter_child_wins_on_the_share_of_its_own_limit_left() {
        let tree = Cgroups::new();
        tree.write("a", &mib(1000).to_string(), 900, 300);
        tree.write("a/b", &mib(200).to_string(), 190, 0);
        tree.put_pid_in(7, "0::/a/b");
        assert_eq!(
            tree.headroom(7),
            Some(Memory {
                total: mib(200),
                available: mib(10)
            })
        );
    }

    #[test]
    fn no_limit_anywhere_is_none() {
        let tree = Cgroups::new();
        tree.write("a/b", "max", 100, 0);
        tree.write("a", "max", 100, 0);
        tree.put_pid_in(7, "0::/a/b");
        assert_eq!(tree.headroom(7), None);
    }

    #[test]
    fn a_cgroup_v1_listing_is_none() {
        let tree = Cgroups::new();
        tree.write("a", &mib(1000).to_string(), 900, 0);
        tree.put_pid_in(8, "4:memory:/a");
        assert_eq!(tree.headroom(8), None);
    }

    #[test]
    fn a_missing_pid_is_none() {
        assert_eq!(Cgroups::new().headroom(9), None);
    }

    #[test]
    fn reads_pss_and_meminfo() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        fs::create_dir_all(root.join("7")).unwrap();
        fs::write(
            root.join("7/smaps_rollup"),
            "00400000-ff Rss: 900 kB\nPss:                 512 kB\nPss_Anon: 100 kB\n",
        )
        .unwrap();
        fs::write(
            root.join("meminfo"),
            "MemTotal:        8000000 kB\nMemFree: 1 kB\nMemAvailable:   2000000 kB\n",
        )
        .unwrap();
        assert_eq!(pss_bytes(root, 7), Some(512 * 1024));
        assert_eq!(pss_bytes(root, 8), None);
        assert_eq!(
            meminfo(root),
            Some(Memory {
                total: 8_000_000 * 1024,
                available: 2_000_000 * 1024
            })
        );
        fs::remove_file(root.join("meminfo")).unwrap();
        assert_eq!(meminfo(root), None);
    }

    #[test]
    fn memory_is_low_under_the_warning_share_and_fine_at_it() {
        let of_1000 = |available| Memory {
            total: 1000,
            available,
        };
        assert!(of_1000(149).below_percent(15));
        assert!(!of_1000(150).below_percent(15));
    }

    #[test]
    fn a_warning_share_of_zero_never_fires() {
        let empty = Memory {
            total: 1000,
            available: 0,
        };
        assert!(!empty.below_percent(0));
    }

    #[test]
    fn the_tighter_memory_is_the_one_with_the_smaller_share_left() {
        let thirty_percent_of_1000 = Memory {
            total: 1_000,
            available: 300,
        };
        let twenty_percent_of_16000 = Memory {
            total: 16_000,
            available: 3_200,
        };
        assert!(twenty_percent_of_16000.tighter_than(thirty_percent_of_1000));
        assert!(!thirty_percent_of_1000.tighter_than(twenty_percent_of_16000));
        assert!(!thirty_percent_of_1000.tighter_than(thirty_percent_of_1000));
    }

    #[test]
    fn reads_the_real_and_effective_uid() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        fs::create_dir_all(root.join("7")).unwrap();
        fs::write(
            root.join("7/status"),
            "Name:\tpython3\nUid:\t1001\t0\t1001\t1001\nGid:\t1001\t1001\t1001\t1001\n",
        )
        .unwrap();
        assert_eq!(uids(root, 7), Some((1001, 0)));
        fs::write(root.join("7/status"), "Name:\tpython3\nUid:\t1001\n").unwrap();
        assert_eq!(uids(root, 7), None);
        assert_eq!(uids(root, 8), None);
    }
}
