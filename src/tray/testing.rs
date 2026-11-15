//! What the tray's tests share: trays in every situation, and walking a
//! built menu as data.

use ksni::menu::MenuItem;

use super::WatchTray;
use super::actions::Driver;
use crate::detect::Memory;
use crate::driver::Verdict;
use crate::sample::Proc;
use crate::sample::testing::proc_row;

pub(super) fn memory(total: u64, available: u64) -> Memory {
    Memory { total, available }
}

pub(super) fn texts(items: Vec<MenuItem<WatchTray>>) -> Vec<String> {
    items
        .into_iter()
        .filter_map(|item| match item {
            MenuItem::Standard(item) => Some(item.label),
            _ => None,
        })
        .collect()
}

pub(super) fn labels(tray: &WatchTray) -> Vec<String> {
    use ksni::Tray;
    texts(tray.menu())
}

pub(super) fn app(pid: u32, name: &str, npu_open: bool) -> Proc {
    Proc {
        name: name.into(),
        cpu_percent: 12.0,
        ram_bytes: 512 * 1024 * 1024,
        npu_open,
        ..proc_row(pid)
    }
}

/// Every situation that changes the icon, tooltip or menu.
pub(super) fn situations() -> Vec<(&'static str, WatchTray)> {
    const GIB: u64 = 1024 * 1024 * 1024;
    let base = || {
        let mut tray = WatchTray::new(None, None);
        tray.snap.memory = Some(memory(16 * GIB, 8 * GIB));
        tray.snap.npu_percent = Some(0.0);
        tray.snap.npu_present = true;
        tray
    };
    let mut all = vec![("idle", base())];
    let mut tray = base();
    tray.snap.npu_percent = None;
    tray.snap.npu_present = false;
    all.push(("no NPU", tray));
    let mut tray = base();
    tray.snap.procs.push(app(7, "vino_speak", true));
    tray.snap.npu_percent = Some(40.0);
    all.push(("active", tray));
    let mut tray = base();
    tray.snap.procs.push(app(7, "vino_speak", false));
    tray.snap.memory = Some(memory(16 * GIB, GIB / 2));
    tray.snap.low_memory = true;
    all.push(("low memory", tray));
    for (name, driver) in [
        ("checking", Driver::Checking),
        ("resetting", Driver::Resetting),
        (
            "healthy",
            Driver::Checked {
                verdict: Verdict::Healthy,
                after_reset: false,
            },
        ),
        (
            "stuck",
            Driver::Checked {
                verdict: Verdict::Stuck,
                after_reset: false,
            },
        ),
        (
            "stuck after reset",
            Driver::Checked {
                verdict: Verdict::Stuck,
                after_reset: true,
            },
        ),
        (
            "inconclusive",
            Driver::Checked {
                verdict: Verdict::Unknown("no_module named openvino".into()),
                after_reset: false,
            },
        ),
        (
            "reset failed",
            Driver::ResetFailed("modprobe failed".into()),
        ),
    ] {
        let mut tray = base();
        tray.driver = driver;
        all.push((name, tray));
    }
    all
}

/// What a screen reader reads: the label without dbusmenu's access-key
/// markers.
pub(super) fn spoken(label: &str) -> String {
    label
        .replace("__", "\u{0}")
        .replace('_', "")
        .replace('\u{0}', "_")
}

/// One item: labelled, shown, and with any underscore kept.
pub(super) fn check(label: &str, visible: bool, out: &mut Vec<String>) {
    assert!(!spoken(label).trim().is_empty(), "unlabelled item");
    assert!(visible, "{label:?} is hidden instead of disabled");
    assert!(
        !label.replace("__", "").contains('_'),
        "{label:?} loses an underscore"
    );
    out.push(spoken(label));
}

/// Every label in a menu, depth first, checking each item on the way.
pub(super) fn walk(items: &[MenuItem<WatchTray>], out: &mut Vec<String>) {
    for item in items {
        match item {
            MenuItem::Standard(i) => check(&i.label, i.visible, out),
            MenuItem::Checkmark(i) => check(&i.label, i.visible, out),
            MenuItem::SubMenu(m) => {
                check(&m.label, m.visible, out);
                walk(&m.submenu, out);
            }
            MenuItem::RadioGroup(g) => g
                .options
                .iter()
                .for_each(|o| check(&o.label, o.visible, out)),
            MenuItem::Separator => {}
        }
    }
}

pub(super) fn all_labels(tray: &WatchTray) -> Vec<String> {
    use ksni::Tray;
    let mut out = Vec::new();
    walk(&tray.menu(), &mut out);
    for reset_installed in [false, true] {
        walk(&tray.driver_items(reset_installed), &mut out);
    }
    out
}
