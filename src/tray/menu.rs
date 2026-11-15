//! The menu, built from the tray's state as plain rows.

use ksni::menu::{MenuItem, StandardItem};
use vino_ui::family::{self, App};
use vino_ui::tray::MenuIcon;

use super::actions::{Driver, Update};
use super::host::menu_text;
use super::{
    CHECK_DRIVER, CHECK_UPDATE, INSTALL_UPDATE, NPU_BUSY_PERCENT, RESET_DRIVER, WatchTray,
};
use crate::driver;
use crate::icon::{self, State};
use crate::sample::Proc;
use crate::update;

/// One process holding this share of RAM gets an amber row: about 3 GiB on a
/// 16 GiB machine. Its RAM meter is full here.
const HEAVY_RAM_SHARE: f64 = 0.2;

/// What the menu offers that depends on the machine, not on a poll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Installed {
    /// `install-npu-reset.sh` has been run, so the reset can start its unit.
    pub(super) reset_unit: bool,
    /// `vinowatch-upgrade` is there, so a newer release can be installed.
    pub(super) upgrade_command: bool,
}

impl Installed {
    pub(super) fn on_this_machine() -> Self {
        Installed {
            reset_unit: driver::reset_installed(),
            upgrade_command: update::upgrade_command().is_some(),
        }
    }
}

enum RowIcon {
    /// From the icon theme.
    Named(&'static str),
    /// PNG.
    Drawn(Vec<u8>),
}

/// A row of text: clicking it does nothing. Left enabled, because hosts grey
/// out a disabled row's icon and the meters are the point.
fn note(label: String, icon: RowIcon) -> MenuItem<WatchTray> {
    let (icon_name, icon_data) = match icon {
        RowIcon::Named(name) => (name.into(), Vec::new()),
        RowIcon::Drawn(png) => (String::new(), png),
    };
    StandardItem {
        label: menu_text(&label),
        icon_name,
        icon_data,
        ..Default::default()
    }
    .into()
}

fn quit_item() -> MenuItem<WatchTray> {
    StandardItem {
        label: "Quit".into(),
        icon_name: MenuIcon::Quit.name().into(),
        activate: Box::new(|_: &mut WatchTray| std::process::exit(0)),
        ..Default::default()
    }
    .into()
}

impl WatchTray {
    /// Every row, top to bottom. `installed` is the only part not read off
    /// the tray's own state, so tests pass it in.
    pub(super) fn build_menu(&self, installed: Installed) -> Vec<MenuItem<Self>> {
        let mut items = self.status_rows();
        items.push(MenuItem::Separator);
        items.extend(self.process_rows());
        items.push(MenuItem::Separator);
        if self.offers_driver_actions() {
            items.extend(self.driver_items(installed.reset_unit));
            items.push(MenuItem::Separator);
        }
        items.push(note(
            format!("vinoWatch {}", env!("CARGO_PKG_VERSION")),
            RowIcon::Named("vinowatch"),
        ));
        items.push(self.update_item(installed.upgrade_command));
        items.push(quit_item());
        items
    }

    /// The tooltip's lines, for hosts that show no tooltip.
    fn status_rows(&self) -> Vec<MenuItem<Self>> {
        let mut rows = vec![
            note(self.summary(), self.npu_icon()),
            note(self.snap.ram_label(), self.ram_icon()),
        ];
        rows.extend(
            self.driver
                .label()
                .map(|label| note(label, RowIcon::Named(self.driver.icon_name()))),
        );
        rows
    }

    fn process_rows(&self) -> Vec<MenuItem<Self>> {
        if self.snap.procs.is_empty() {
            let empty = RowIcon::Drawn(icon::meter_png(State::Idle, &[0.0; 3]));
            return vec![note("No OpenVINO processes".into(), empty)];
        }
        self.snap
            .procs
            .iter()
            .map(|p| note(p.label(), self.proc_icon(p)))
            .collect()
    }

    /// The update row under the version. Without `vinowatch-upgrade` (a build
    /// installed by hand) a newer release opens its page instead.
    pub(super) fn update_item(&self, can_install: bool) -> MenuItem<Self> {
        let activate: Box<dyn Fn(&mut Self) + Send> = match &self.update {
            Update::Available(tag) if !can_install => {
                let url = format!(
                    "https://github.com/{}/releases/tag/{tag}",
                    App::Watch.repo()
                );
                Box::new(move |_: &mut Self| {
                    if let Err(e) = family::open(&url) {
                        eprintln!("vinowatch: couldn't open {url}: {e}");
                    }
                })
            }
            Update::Available(_) => Box::new(|tray: &mut Self| tray.start(INSTALL_UPDATE)),
            _ => Box::new(|tray: &mut Self| tray.start(CHECK_UPDATE)),
        };
        StandardItem {
            label: menu_text(&self.update.label()),
            icon_name: MenuIcon::Update.name().into(),
            enabled: !self.update.busy(),
            activate,
            ..Default::default()
        }
        .into()
    }

    /// Processes with `/dev/accel` open: `modprobe -r` refuses while any do.
    fn holders(&self) -> impl Iterator<Item = &Proc> {
        self.snap.procs.iter().filter(|p| p.npu_open)
    }

    /// The NPU meter: green while it's busy.
    fn npu_icon(&self) -> RowIcon {
        let percent = self.snap.npu_percent.unwrap_or(0.0);
        let state = if percent >= NPU_BUSY_PERCENT {
            State::Active
        } else {
            State::Idle
        };
        RowIcon::Drawn(icon::meter_png(state, &[(percent / 100.0) as f32]))
    }

    /// How full RAM is: amber when it's low, green while OpenVINO runs.
    fn ram_icon(&self) -> RowIcon {
        let used = self.snap.memory.map_or(0.0, |m| 1.0 - m.share_left());
        let state = if self.snap.low_memory {
            State::LowMemory
        } else if self.in_use() {
            State::Active
        } else {
            State::Idle
        };
        RowIcon::Drawn(icon::meter_png(state, &[used as f32]))
    }

    fn ram_share(&self, p: &Proc) -> f64 {
        self.snap
            .memory
            .map_or(0.0, |m| p.ram_bytes as f64 / m.total.max(1) as f64)
    }

    /// Amber when it's the heavy one (or the biggest while RAM is low), green
    /// while it's working, white while it waits.
    pub(super) fn proc_state(&self, p: &Proc) -> State {
        let biggest = self.snap.procs.iter().map(|q| q.ram_bytes).max();
        if self.ram_share(p) >= HEAVY_RAM_SHARE
            || (self.snap.low_memory && biggest == Some(p.ram_bytes))
        {
            State::LowMemory
        } else if p.cpu_percent >= 1.0
            || (p.npu_open && self.snap.npu_percent.is_some_and(|n| n >= NPU_BUSY_PERCENT))
        {
            State::Active
        } else {
            State::Idle
        }
    }

    /// The process's own CPU, RAM and NPU meters. A full CPU meter is one
    /// core; a full RAM meter is `HEAVY_RAM_SHARE`.
    fn proc_icon(&self, p: &Proc) -> RowIcon {
        let levels = [
            p.cpu_percent / 100.0,
            (self.ram_share(p) / HEAVY_RAM_SHARE) as f32,
            if p.npu_open { 1.0 } else { 0.0 },
        ];
        RowIcon::Drawn(icon::meter_png(self.proc_state(p), &levels))
    }

    /// With an NPU, and from the first check on even without one: a reload
    /// whose `modprobe` failed leaves no device, and the reset is the retry
    /// (`the_driver_check_shows_with_an_npu`).
    fn offers_driver_actions(&self) -> bool {
        self.snap.npu_present || self.driver != Driver::Unchecked
    }

    /// `reset_installed`: whether `install-npu-reset.sh` has been run.
    pub(super) fn driver_items(&self, reset_installed: bool) -> Vec<MenuItem<Self>> {
        let busy = self.driver.busy();
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: "Check NPU driver".into(),
                icon_name: MenuIcon::Search.name().into(),
                enabled: !busy,
                activate: Box::new(|tray: &mut Self| tray.start(CHECK_DRIVER)),
                ..Default::default()
            }
            .into(),
        ];
        if !reset_installed {
            if self.driver.stuck() {
                items.push(note(
                    "Reset needs: sudo ./install-npu-reset.sh".into(),
                    RowIcon::Named(MenuIcon::Info.name()),
                ));
            }
            return items;
        }
        let holders: Vec<&Proc> = self.holders().collect();
        items.push(
            StandardItem {
                label: "Reset NPU driver".into(),
                icon_name: MenuIcon::Refresh.name().into(),
                enabled: !busy && holders.is_empty(),
                activate: Box::new(|tray: &mut Self| tray.start(RESET_DRIVER)),
                ..Default::default()
            }
            .into(),
        );
        if !holders.is_empty() {
            items.push(note(
                "Close these first:".into(),
                RowIcon::Named(MenuIcon::Warning.name()),
            ));
            items.extend(self.holders().map(|p| note(p.label(), self.proc_icon(p))));
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{all_labels, labels, memory, situations, texts};
    use super::*;
    use crate::driver::Verdict;
    use crate::sample::testing::proc_row;

    #[test]
    fn the_menu_ends_with_quit_and_lists_no_other_apps() {
        let menu = ksni::Tray::menu(&WatchTray::new(None, None));
        let [.., MenuItem::Standard(quit)] = menu.as_slice() else {
            panic!("the menu should end with Quit");
        };
        assert_eq!(quit.label, "Quit");
        fn labels(items: &[MenuItem<WatchTray>], out: &mut Vec<String>) {
            for item in items {
                match item {
                    MenuItem::Standard(item) => out.push(item.label.clone()),
                    MenuItem::SubMenu(item) => {
                        out.push(item.label.clone());
                        labels(&item.submenu, out);
                    }
                    _ => {}
                }
            }
        }
        let mut all = Vec::new();
        labels(&menu, &mut all);
        assert!(
            !all.iter().any(|label| label.contains("vinoPilot")
                || label.contains(family::SITE_LABEL)
                || label == "vinoWhisper"),
            "{all:?}"
        );
    }

    #[test]
    fn a_process_row_is_coloured_by_what_it_is_doing() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let proc = |pid, cpu_percent, ram_bytes, npu_open| Proc {
            cpu_percent,
            ram_bytes,
            npu_open,
            ..proc_row(pid)
        };
        let mut tray = WatchTray::new(None, None);
        tray.snap.memory = Some(memory(16 * GIB, 8 * GIB));
        tray.snap.npu_percent = Some(0.0);
        let waiting = proc(1, 0.2, GIB, true);
        let working = proc(2, 40.0, GIB / 2, false);
        let heavy = proc(3, 0.0, 4 * GIB, false);
        tray.snap.procs = vec![waiting.clone(), working.clone(), heavy.clone()];
        assert_eq!(tray.proc_state(&waiting), State::Idle);
        assert_eq!(tray.proc_state(&working), State::Active);
        assert_eq!(tray.proc_state(&heavy), State::LowMemory);
        // Holding the NPU counts as work once the NPU is busy.
        tray.snap.npu_percent = Some(30.0);
        assert_eq!(tray.proc_state(&waiting), State::Active);
        // Low RAM points at the biggest process even under the threshold.
        tray.snap.procs = vec![waiting.clone(), working.clone()];
        tray.snap.low_memory = true;
        assert_eq!(tray.proc_state(&waiting), State::LowMemory);
        assert_eq!(tray.proc_state(&working), State::Active);
    }

    #[test]
    fn every_row_has_an_icon() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        tray.snap.npu_present = true;
        tray.snap.procs.push(Proc {
            npu_open: true,
            ..proc_row(1)
        });
        tray.driver = Driver::Checked {
            verdict: Verdict::Stuck,
            after_reset: false,
        };
        for reset_installed in [false, true] {
            let mut items = tray.menu();
            items.extend(tray.driver_items(reset_installed));
            for item in items {
                if let MenuItem::Standard(item) = item {
                    assert!(
                        !item.icon_name.is_empty() || !item.icon_data.is_empty(),
                        "{} has no icon",
                        item.label
                    );
                    assert!(
                        item.enabled || item.label.contains("NPU driver"),
                        "{} greys its icon",
                        item.label
                    );
                }
            }
        }
    }

    fn update_label(tray: &WatchTray, can_install: bool) -> (String, bool) {
        match tray.update_item(can_install) {
            MenuItem::Standard(item) => (item.label, item.enabled),
            _ => panic!("the update row isn't a plain item"),
        }
    }

    #[test]
    fn the_update_row_says_where_it_is() {
        let mut tray = WatchTray::new(None, None);
        assert_eq!(
            update_label(&tray, true),
            ("Check for updates".into(), true)
        );
        assert!(labels(&tray).contains(&"Check for updates".to_string()));
        tray.update = Update::Checking;
        assert_eq!(
            update_label(&tray, true),
            ("Checking for updates…".into(), false)
        );
        tray.update = Update::Available("v0.5.0".into());
        assert_eq!(update_label(&tray, true), ("Update to v0.5.0".into(), true));
        // A build installed by hand still hears about it, and opens the page.
        assert_eq!(
            update_label(&tray, false),
            ("Update to v0.5.0".into(), true)
        );
        tray.update = Update::Installing("v0.5.0".into());
        assert_eq!(
            update_label(&tray, true),
            ("Installing v0.5.0…".into(), false)
        );
        tray.update = Update::Failed("couldn't reach GitHub".into());
        assert_eq!(
            update_label(&tray, true),
            (
                "Update failed (couldn't reach GitHub): try again".into(),
                true
            )
        );
    }

    #[test]
    fn the_driver_check_shows_with_an_npu() {
        let mut tray = WatchTray::new(None, None);
        assert!(!labels(&tray).contains(&"Check NPU driver".to_string()));
        tray.snap.npu_present = true;
        assert!(labels(&tray).contains(&"Check NPU driver".to_string()));
        // A reload that leaves no device must still offer a retry.
        tray.snap.npu_present = false;
        tray.driver = Driver::ResetFailed("modprobe failed".into());
        assert!(labels(&tray).contains(&"Check NPU driver".to_string()));
        assert!(labels(&tray).contains(&"NPU reset failed: modprobe failed".to_string()));
    }

    #[test]
    fn the_reset_waits_for_the_npu_holders() {
        let mut tray = WatchTray::new(None, None);
        tray.driver = Driver::Checked {
            verdict: Verdict::Stuck,
            after_reset: false,
        };
        assert_eq!(
            texts(tray.driver_items(false)),
            [
                "Check NPU driver",
                "Reset needs: sudo ./install-npu-reset.sh"
            ]
        );
        let enabled = |tray: &WatchTray| {
            tray.driver_items(true)
                .into_iter()
                .find_map(|item| match item {
                    MenuItem::Standard(i) if i.label == "Reset NPU driver" => Some(i.enabled),
                    _ => None,
                })
        };
        assert_eq!(enabled(&tray), Some(true));
        tray.snap.procs.push(Proc {
            name: "vino_speak".into(),
            npu_open: true,
            ..proc_row(7)
        });
        assert_eq!(enabled(&tray), Some(false));
        let items = texts(tray.driver_items(true));
        assert_eq!(items[2], "Close these first:");
        assert!(items[3].starts_with("vino__speak pid 7"), "{}", items[3]);
        tray.driver = Driver::Resetting;
        assert_eq!(enabled(&tray), Some(false));
    }

    #[test]
    fn every_menu_item_has_a_label() {
        for (name, tray) in situations() {
            let labels = all_labels(&tray);
            assert!(labels.contains(&"Quit".to_string()), "{name}: {labels:?}");
        }
        let (_, active) = situations().swap_remove(2);
        assert!(
            all_labels(&active)
                .iter()
                .any(|l| l.starts_with("vino_speak pid 7")),
            "the underscore in vino_speak is lost"
        );
    }

    #[test]
    fn a_running_check_greys_the_actions_and_says_why() {
        use ksni::Tray;
        for (name, tray) in situations() {
            let busy = tray.driver.busy();
            let menu = tray.menu();
            for item in tray.driver_items(true) {
                if let MenuItem::Standard(i) = item
                    && i.label == "Check NPU driver"
                {
                    assert_eq!(i.enabled, !busy, "{name}");
                }
            }
            if busy {
                let MenuItem::Standard(first) = &menu[2] else {
                    panic!("{name}: no driver row")
                };
                assert!(first.label.ends_with('…'), "{name}: {}", first.label);
            }
        }
    }

    const EVERYTHING: Installed = Installed {
        reset_unit: true,
        upgrade_command: true,
    };
    const NOTHING: Installed = Installed {
        reset_unit: false,
        upgrade_command: false,
    };

    /// vinoSpeak on a busy NPU, which it holds.
    fn active() -> WatchTray {
        let (name, tray) = situations().swap_remove(2);
        assert_eq!(name, "active");
        tray
    }

    #[test]
    fn the_menu_reads_status_then_processes_then_actions_then_about() {
        let speak = "vino__speak pid 7: CPU 12.0%, RAM 512 MiB, NPU open";
        assert_eq!(
            texts(active().build_menu(EVERYTHING)),
            [
                "NPU 40% · 1 OpenVINO process",
                "OpenVINO apps hold 0.5 GiB of 16.0 GiB RAM, 8.0 GiB available",
                speak,
                "Check NPU driver",
                "Reset NPU driver",
                "Close these first:",
                speak,
                concat!("vinoWatch ", env!("CARGO_PKG_VERSION")),
                "Check for updates",
                "Quit",
            ]
        );
    }

    #[test]
    fn without_the_reset_unit_the_menu_offers_only_the_check() {
        let rows = texts(active().build_menu(NOTHING));
        assert!(rows.contains(&"Check NPU driver".to_string()));
        assert!(!rows.contains(&"Reset NPU driver".to_string()));
    }

    #[test]
    fn without_an_npu_the_menu_has_no_driver_section() {
        let separators = |tray: &WatchTray| {
            tray.build_menu(EVERYTHING)
                .iter()
                .filter(|item| matches!(item, MenuItem::Separator))
                .count()
        };
        let mut tray = active();
        assert_eq!(separators(&tray), 3);
        tray.snap.npu_present = false;
        assert_eq!(separators(&tray), 2);
        assert!(!texts(tray.build_menu(EVERYTHING)).contains(&"Check NPU driver".to_string()));
    }

    #[test]
    fn an_empty_list_says_so_in_one_row() {
        let rows = texts(WatchTray::new(None, None).build_menu(NOTHING));
        assert_eq!(rows[2], "No OpenVINO processes");
        assert_eq!(rows.len(), 6);
    }
}
