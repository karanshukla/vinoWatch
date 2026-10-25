//! Deciding which processes are OpenVINO users, and which vino app they belong to.
//!
//! Everything takes the `/proc` root as a parameter so the tests can build a
//! fake one.

use std::fmt;
use std::fs;
use std::path::Path;

/// The vino apps that run models through OpenVINO.
// The prefix is the apps' own name, and Display prints it back.
#[allow(clippy::enum_variant_names)]
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
/// directory.
///
/// The kernel truncates `comm` to 15 characters (`vinowhisper-serv`) and the
/// Python apps all show up as `python3`, so the command line is what names them.
pub fn classify(cmdline: &[String], cwd: Option<&str>) -> Option<Project> {
    let mut haystack = cmdline.join(" ").to_lowercase();
    // vinoPop runs as `python3 <checkout>/vinopop` (vinoPop#70), or as
    // `python3 main.py` from older launchers; then its checkout path, in the
    // command line or the working directory, is what names it.
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

/// Whether a `/proc/<pid>/maps` listing maps an OpenVINO library.
///
/// Only the file name counts, so a data file that happens to have
/// `libopenvino` somewhere in its path doesn't.
pub fn maps_openvino(maps: &str) -> bool {
    maps.lines().any(|line| {
        // address perms offset dev inode path; the path may contain spaces.
        let Some(path) = line.splitn(6, char::is_whitespace).nth(5) else {
            return false;
        };
        let path = path.trim().trim_end_matches(" (deleted)");
        path.rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("libopenvino"))
    })
}

/// Whether `pid` has an OpenVINO library mapped. `None` when we may not read
/// the maps file: other users' processes, and setuid or setgid ones like
/// `vinoauthface-auth`, which the kernel makes non-dumpable. A process that
/// has gone is `Some(false)`.
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

/// `(MemTotal, MemAvailable)` in bytes from `/proc/meminfo`.
pub fn meminfo(proc_root: &Path) -> Option<(u64, u64)> {
    let text = fs::read_to_string(proc_root.join("meminfo")).ok()?;
    Some((
        kib_field(&text, "MemTotal:")? * 1024,
        kib_field(&text, "MemAvailable:")? * 1024,
    ))
}

/// The tightest memory limit above `pid` in the cgroup v2 tree, as
/// `(limit, available)` bytes, or `None` if no cgroup around it sets one (or
/// the tree isn't cgroup v2).
///
/// "Used" is `memory.current` minus inactive file cache, like `docker stats`:
/// the cgroup is charged for page cache too, but that gives way under
/// pressure, so counting it would raise the alarm too early.
pub fn cgroup_headroom(proc_root: &Path, cgroup_root: &Path, pid: u32) -> Option<(u64, u64)> {
    let listing = fs::read_to_string(proc_root.join(format!("{pid}/cgroup"))).ok()?;
    // cgroup v2 is the single `0::/path` line; v1 has one line per controller.
    let path = listing.lines().find_map(|l| l.strip_prefix("0::"))?;
    let mut dir = cgroup_root.join(path.trim_start_matches('/'));
    let mut tightest: Option<(u64, u64)> = None;
    loop {
        if let Some(limit) = read_u64(&dir.join("memory.max")) {
            let current = read_u64(&dir.join("memory.current")).unwrap_or(0);
            let cache = fs::read_to_string(dir.join("memory.stat"))
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("inactive_file "))
                        .and_then(|n| n.trim().parse().ok())
                })
                .unwrap_or(0);
            let available = limit.saturating_sub(current.saturating_sub(cache));
            // Smallest share of its own limit left is the one that binds.
            let share = |(l, a): (u64, u64)| a as f64 / l.max(1) as f64;
            if tightest.is_none_or(|t| share((limit, available)) < share(t)) {
                tightest = Some((limit, available));
            }
        }
        if dir == cgroup_root || !dir.pop() || !dir.starts_with(cgroup_root) {
            return tightest;
        }
    }
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
        let root = std::env::temp_dir().join(format!("vinowatch-detect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
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

        assert_eq!(uses_openvino(&root, 7), Some(true));
        assert!(holds_npu(&root, 7));
        assert_eq!(uses_openvino(&root, 8), Some(false));
        assert!(!holds_npu(&root, 8));
        // A pid with no /proc entry is "no"; one we may not read, "can't tell".
        assert_eq!(uses_openvino(&root, 9), Some(false));
        fs::create_dir_all(root.join("10")).unwrap();
        fs::write(root.join("10/maps"), "").unwrap();
        fs::set_permissions(root.join("10/maps"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(root.join("10/maps")).is_err() {
            // Root reads it anyway.
            assert_eq!(uses_openvino(&root, 10), None);
        }
        assert!(!holds_npu(&root, 9));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_the_tightest_cgroup_limit() {
        let root = std::env::temp_dir().join(format!("vinowatch-cg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (proc_root, cg) = (root.join("proc"), root.join("cg"));
        let mib = |n: u64| (n * 1024 * 1024).to_string();
        let write = |dir: &str, max: &str, current: u64, inactive: u64| {
            let d = cg.join(dir);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("memory.max"), format!("{max}\n")).unwrap();
            fs::write(d.join("memory.current"), mib(current)).unwrap();
            fs::write(
                d.join("memory.stat"),
                format!("anon 1\ninactive_file {}\n", mib(inactive)),
            )
            .unwrap();
        };
        // The app's own cgroup is unlimited; its parent has 1000 MiB with 900
        // used, 300 of which is inactive cache; the root has none.
        write("a/b", "max", 100, 0);
        write("a", &mib(1000), 900, 300);
        fs::create_dir_all(proc_root.join("7")).unwrap();
        fs::write(proc_root.join("7/cgroup"), "0::/a/b\n").unwrap();

        assert_eq!(
            cgroup_headroom(&proc_root, &cg, 7),
            Some((1000 << 20, 400 << 20))
        );

        // A tighter child wins on the share of its own limit left.
        write("a/b", &mib(200), 190, 0);
        assert_eq!(
            cgroup_headroom(&proc_root, &cg, 7),
            Some((200 << 20, 10 << 20))
        );

        // No limit anywhere, a v1 listing, and a missing pid are all "none".
        fs::remove_dir_all(cg.join("a")).unwrap();
        fs::create_dir_all(cg.join("a/b")).unwrap();
        assert_eq!(cgroup_headroom(&proc_root, &cg, 7), None);
        fs::create_dir_all(proc_root.join("8")).unwrap();
        fs::write(proc_root.join("8/cgroup"), "4:memory:/a\n").unwrap();
        assert_eq!(cgroup_headroom(&proc_root, &cg, 8), None);
        assert_eq!(cgroup_headroom(&proc_root, &cg, 9), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reads_pss_and_meminfo() {
        let root = std::env::temp_dir().join(format!("vinowatch-mem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
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
        assert_eq!(pss_bytes(&root, 7), Some(512 * 1024));
        assert_eq!(pss_bytes(&root, 8), None);
        assert_eq!(meminfo(&root), Some((8_000_000 * 1024, 2_000_000 * 1024)));
        fs::remove_file(root.join("meminfo")).unwrap();
        assert_eq!(meminfo(&root), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reads_the_real_and_effective_uid() {
        let root = std::env::temp_dir().join(format!("vinowatch-uid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("7")).unwrap();
        fs::write(
            root.join("7/status"),
            "Name:\tpython3\nUid:\t1001\t0\t1001\t1001\nGid:\t1001\t1001\t1001\t1001\n",
        )
        .unwrap();
        assert_eq!(uids(&root, 7), Some((1001, 0)));
        fs::write(root.join("7/status"), "Name:\tpython3\nUid:\t1001\n").unwrap();
        assert_eq!(uids(&root, 7), None);
        assert_eq!(uids(&root, 8), None);
        fs::remove_dir_all(&root).unwrap();
    }
}
