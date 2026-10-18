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
}

impl fmt::Display for Project {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::VinoWhisper => "vinoWhisper",
            Self::VinoSpeak => "vinoSpeak",
            Self::VinoPop => "vinoPop",
            Self::VinoAuthFace => "vinoAuthFace",
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
    // vinoPop is `python3 main.py`; only its checkout directory says so.
    if let Some(cwd) = cwd {
        haystack.push(' ');
        haystack.push_str(&cwd.to_lowercase());
    }
    const MARKERS: [(&str, Project); 6] = [
        ("vinowhisper", Project::VinoWhisper),
        ("vinospeak", Project::VinoSpeak),
        ("vino_speak", Project::VinoSpeak),
        ("vinoauthface", Project::VinoAuthFace),
        ("face-auth", Project::VinoAuthFace),
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

/// Whether `pid` has an OpenVINO library mapped. `false` when the maps file
/// can't be read, which is the case for other users' processes.
pub fn uses_openvino(proc_root: &Path, pid: u32) -> bool {
    fs::read_to_string(proc_root.join(format!("{pid}/maps")))
        .map(|maps| maps_openvino(&maps))
        .unwrap_or(false)
}

/// Whether `pid` has an `/dev/accel/*` node open, i.e. it is talking to the NPU.
pub fn holds_npu(proc_root: &Path, pid: u32) -> bool {
    let Ok(fds) = fs::read_dir(proc_root.join(format!("{pid}/fd"))) else {
        return false;
    };
    fds.flatten()
        .any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target.starts_with("/dev/accel")))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ];
        for (argv, want) in cases {
            assert_eq!(classify(&args(argv), None), Some(want), "{argv:?}");
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

        assert!(uses_openvino(&root, 7));
        assert!(holds_npu(&root, 7));
        assert!(!uses_openvino(&root, 8));
        assert!(!holds_npu(&root, 8));
        // A pid with no /proc entry, or one we may not read, is just "no".
        assert!(!uses_openvino(&root, 9));
        assert!(!holds_npu(&root, 9));
        fs::remove_dir_all(&root).unwrap();
    }
}
