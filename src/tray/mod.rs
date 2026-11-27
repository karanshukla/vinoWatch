//! The tray icon: the NPU load in the tooltip, one menu row per process.

mod actions;
mod host;
mod jobs;
mod menu;
#[cfg(test)]
mod testing;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ksni::menu::MenuItem;
use ksni::{Category, ToolTip, TrayMethods};

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::driver;
use crate::icon::{self, State};
use crate::sample::{Sampler, Snapshot};

use actions::{Driver, DriverJob, Job, Update, UpdateJob};
use host::{HostTracker, markup_escape, watcher_host};
use menu::Installed;

/// Below this the NPU counts as idle; the busy counter ticks a little even
/// between inferences.
const NPU_BUSY_PERCENT: f64 = 1.0;

/// The icon goes passive once the NPU has been idle this long, as the other
/// vino trays do after their idle minutes.
const HIDE_AFTER: Duration = Duration::from_secs(5 * 60);

const TRAY_GONE: &str = "the tray icon disconnected";

const CHECK_DRIVER: Job = Job::Driver(DriverJob::Check);
const RESET_DRIVER: Job = Job::Driver(DriverJob::Reset);
const CHECK_UPDATE: Job = Job::Update(UpdateJob::Check);
const INSTALL_UPDATE: Job = Job::Update(UpdateJob::Install);

struct WatchTray {
    snap: Snapshot,
    driver: Driver,
    /// `--probe-python`, else picked from `seen_pythons`.
    probe_python: Option<PathBuf>,
    /// Every vino app interpreter seen this session, newest last. Kept after
    /// the apps exit: a reset needs them closed, and the check after it still
    /// needs an interpreter with openvino.
    seen_pythons: Vec<PathBuf>,
    jobs: Option<UnboundedSender<Job>>,
    update: Update,
    /// The last poll with the NPU busy, or startup.
    npu_used: Instant,
    /// `HIDE_AFTER` has passed since `npu_used`.
    npu_quiet: bool,
    /// Send the themed `-symbolic` icon names. Off for hosts that draw them
    /// without recolouring (see `host_recolours`), which then get the pixmaps.
    themed_icons: bool,
}

impl WatchTray {
    fn new(probe_python: Option<PathBuf>, jobs: Option<UnboundedSender<Job>>) -> Self {
        WatchTray {
            snap: Snapshot::default(),
            driver: Driver::default(),
            probe_python,
            seen_pythons: Vec::new(),
            jobs,
            update: Update::default(),
            npu_used: Instant::now(),
            npu_quiet: false,
            themed_icons: true,
        }
    }

    fn set_snapshot(&mut self, snap: Snapshot) {
        self.set_snapshot_at(snap, Instant::now());
    }

    fn set_snapshot_at(&mut self, snap: Snapshot, now: Instant) {
        if snap.npu_percent.is_some_and(|p| p >= NPU_BUSY_PERCENT) {
            self.npu_used = now;
        }
        self.npu_quiet = now.saturating_duration_since(self.npu_used) >= HIDE_AFTER;
        for python in &snap.pythons {
            self.seen_pythons.retain(|p| p != python);
            self.seen_pythons.push(python.clone());
        }
        self.snap = snap;
    }

    fn python(&self) -> PathBuf {
        driver::pick_python(self.probe_python.as_deref(), &self.seen_pythons)
    }

    /// Starts `job` unless one of its kind is already running.
    fn start(&mut self, job: Job) {
        let Some(jobs) = &self.jobs else { return };
        let busy = match job {
            Job::Driver(_) => self.driver.busy(),
            Job::Update(_) => self.update.busy(),
        };
        if busy {
            return;
        }
        if job == INSTALL_UPDATE && !matches!(self.update, Update::Available(_)) {
            return;
        }
        if jobs.send(job).is_err() {
            return;
        }
        match job {
            Job::Driver(DriverJob::Check) => self.driver = Driver::Checking,
            Job::Driver(DriverJob::Reset) => self.driver = Driver::Resetting,
            Job::Update(UpdateJob::Check) => self.update = Update::Checking,
            Job::Update(UpdateJob::Install) => {
                if let Update::Available(tag) = &self.update {
                    self.update = Update::Installing(tag.clone());
                }
            }
        }
    }

    fn summary(&self) -> String {
        let apps = self.snap.procs.iter().filter(|p| p.openvino).count();
        let helpers = self.snap.procs.len() - apps;
        let npu = self.snap.npu_label();
        match (apps, helpers) {
            (0, 0) => format!("{npu} · Idle"),
            (0, 1) => format!("{npu} · 1 helper"),
            (0, h) => format!("{npu} · {h} helpers"),
            (a, 0) => format!("{npu} · {a} app{}", if a == 1 { "" } else { "s" }),
            (a, h) => format!(
                "{npu} · {a} app{} + {h} helper{}",
                if a == 1 { "" } else { "s" },
                if h == 1 { "" } else { "s" },
            ),
        }
    }

    pub(super) fn ram_summary(&self) -> String {
        const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
        let gib = |bytes: u64| bytes as f64 / GIB;
        let used = gib(self.snap.openvino_bytes());
        match self.snap.memory {
            Some(crate::detect::Memory { total, available }) => format!(
                "OpenVINO: {used:.1} GiB RAM · {:.1} / {:.1} GiB {}free{}",
                gib(available),
                gib(total),
                if self.snap.cgroup_limited {
                    "cgroup "
                } else {
                    ""
                },
                if self.snap.low_memory { " (low)" } else { "" },
            ),
            None => format!("OpenVINO: {used:.1} GiB RAM"),
        }
    }

    /// Something is running on OpenVINO: a listed process that loads it, or
    /// NPU load from one we couldn't list (another user's, or gone before the
    /// poll saw it). A vino app's helper alone (its tray, say) doesn't count.
    fn in_use(&self) -> bool {
        self.snap.procs.iter().any(|p| p.openvino)
            || self.snap.npu_percent.is_some_and(|p| p >= NPU_BUSY_PERCENT)
    }

    fn state(&self) -> State {
        if self.snap.low_memory || self.driver.stuck() {
            State::Warning
        } else if self.in_use() {
            State::Active
        } else {
            State::Idle
        }
    }
}

/// The drawn icons for one state: the fallback when the icon theme has no
/// `vinowatch-*-symbolic` (it isn't installed, or the host ignores themes).
fn pixmaps(state: State) -> Vec<ksni::Icon> {
    static ICONS: OnceLock<[Vec<ksni::Icon>; 3]> = OnceLock::new();
    let all = ICONS.get_or_init(|| State::ALL.map(icon::tray_icons));
    let i = State::ALL.iter().position(|&s| s == state).unwrap_or(0);
    all[i].clone()
}

impl ksni::Tray for WatchTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "vinoWatch".into()
    }

    // Plasma files Hardware items in a different part of the panel.
    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn icon_name(&self) -> String {
        if !self.themed_icons {
            return String::new();
        }
        self.state().icon_name().into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(self.state())
    }

    /// Hosts hide a Passive item
    /// (pinned by `tests::the_icon_hides_at_five_quiet_minutes`,
    /// `tests::a_stuck_driver_turns_amber_and_stays_in_view`).
    fn status(&self) -> ksni::Status {
        if self.driver.stuck() {
            ksni::Status::NeedsAttention
        } else if self.npu_quiet && !self.driver.busy() {
            ksni::Status::Passive
        } else if self.snap.low_memory && self.in_use() {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }

    // Keep running: sway starts waybar after the unit, and exiting trips its restart limit.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        let why = match reason {
            ksni::OfflineReason::Error(e) => e.to_string(),
            _ => "tray host disconnected".into(),
        };
        eprintln!(
            "vinowatch: no tray host available ({why}); waiting for one to start \
             (GNOME requires the AppIndicator extension)"
        );
        true
    }

    fn watcher_online(&self) {
        eprintln!("vinowatch: tray host connected; icon restored");
    }

    fn attention_icon_name(&self) -> String {
        if !self.themed_icons {
            return String::new();
        }
        State::Warning.icon_name().into()
    }

    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(State::Warning)
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "vinoWatch".into(),
            description: [
                Some(self.summary()),
                Some(self.ram_summary()),
                self.driver.label(),
            ]
            .into_iter()
            .flatten()
            .map(|line| markup_escape(&line))
            .collect::<Vec<_>>()
            .join("\n"),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        self.build_menu(Installed::on_this_machine())
    }
}

/// Runs until the tray is quit. Errors if there is no session bus; with no
/// tray host on it yet, it waits for one.
pub fn run(
    interval: Duration,
    warn_below: u8,
    probe_python: Option<PathBuf>,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let mut sampler = Sampler::started(warn_below);
        let (jobs, mut queue) = unbounded_channel();
        let handle = WatchTray::new(probe_python, Some(jobs))
            .assume_sni_available(true)
            .spawn()
            .await
            .map_err(|e| format!("couldn't register the tray icon: {e}"))?;
        let bus = zbus::Connection::session().await.ok();
        let dbus = match &bus {
            Some(bus) => zbus::fdo::DBusProxy::new(bus).await.ok(),
            None => None,
        };
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let mut host = HostTracker::default();
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticks.tick() => {
                    if let Some(dbus) = &dbus {
                        let watcher = watcher_host(dbus).await;
                        if let Some(themed) = host.observe(watcher, desktop.as_deref()) {
                            handle.update(|tray| tray.themed_icons = themed).await;
                        }
                    }
                    let snap = sampler.poll();
                    if handle.update(|tray| tray.set_snapshot(snap)).await.is_none() {
                        return Err(TRAY_GONE.into());
                    }
                }
                Some(job) = queue.recv() => {
                    if jobs::dispatch(job, &handle).await.is_err() {
                        return Err(TRAY_GONE.into());
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::testing::all_labels;
    use super::testing::situations;
    use super::*;
    use crate::driver::Verdict;
    use crate::sample::Proc;
    use crate::sample::testing::proc_row;
    use std::collections::HashMap;

    #[test]
    fn hosts_that_dont_recolour_get_pixmaps() {
        let mut tray = WatchTray::new(None, None);
        tray.themed_icons = false;
        assert_eq!(ksni::Tray::icon_name(&tray), "");
        assert_eq!(ksni::Tray::attention_icon_name(&tray), "");
        assert!(!ksni::Tray::icon_pixmap(&tray).is_empty());
    }

    #[test]
    fn an_openvino_process_makes_the_icon_active() {
        let mut tray = WatchTray::new(None, None);
        assert_eq!(tray.state(), State::Idle);
        tray.snap.procs.push(proc_row(1));
        assert_eq!(tray.state(), State::Active);
    }

    #[test]
    fn a_vino_apps_helper_with_no_openvino_is_listed_not_active() {
        let mut tray = WatchTray::new(None, None);
        tray.snap.procs.push(Proc {
            openvino: false,
            ..proc_row(1)
        });
        assert_eq!(tray.state(), State::Idle);
        assert!(tray.summary().ends_with("· 1 helper"));
    }

    #[test]
    fn low_memory_turns_the_icon_amber() {
        let mut tray = WatchTray::new(None, None);
        tray.snap.low_memory = true;
        assert_eq!(tray.state(), State::Warning);
        assert_eq!(pixmaps(State::Warning).len(), 6);
    }

    #[test]
    fn npu_load_from_a_process_we_couldnt_list_still_counts() {
        let mut tray = WatchTray::new(None, None);
        tray.snap.npu_percent = Some(0.4);
        assert_eq!(tray.state(), State::Idle);
        tray.snap.npu_percent = Some(30.0);
        assert_eq!(tray.state(), State::Active);
    }

    fn npu_at(percent: f64) -> Snapshot {
        Snapshot {
            npu_percent: Some(percent),
            ..Default::default()
        }
    }

    fn resident_app() -> Proc {
        Proc {
            name: "vino_speak".into(),
            cpu_percent: 3.0,
            ram_bytes: 1,
            npu_open: true,
            ..proc_row(1)
        }
    }

    #[test]
    fn the_icon_is_in_view_just_before_five_quiet_minutes() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        let start = tray.npu_used;
        tray.set_snapshot_at(npu_at(0.0), start + HIDE_AFTER - Duration::from_secs(1));
        assert_eq!(tray.status(), ksni::Status::Active);
    }

    #[test]
    fn the_icon_hides_at_five_quiet_minutes() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        let start = tray.npu_used;
        tray.set_snapshot_at(npu_at(0.4), start + HIDE_AFTER);
        assert_eq!(tray.status(), ksni::Status::Passive);
    }

    #[test]
    fn low_memory_alone_does_not_bring_a_hidden_icon_back() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        let start = tray.npu_used;
        tray.snap.low_memory = true;
        tray.set_snapshot_at(npu_at(0.0), start + HIDE_AFTER);
        assert_eq!(tray.status(), ksni::Status::Passive);
    }

    #[test]
    fn a_resident_app_with_the_npu_idle_does_not_bring_it_back() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        let start = tray.npu_used;
        tray.set_snapshot_at(npu_at(0.0), start + HIDE_AFTER);
        tray.snap.procs.push(resident_app());
        assert_eq!(tray.status(), ksni::Status::Passive);
    }

    #[test]
    fn npu_use_brings_the_icon_back_for_another_five_minutes() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        let used = tray.npu_used + HIDE_AFTER * 2;
        tray.set_snapshot_at(npu_at(30.0), used);
        assert_eq!(tray.status(), ksni::Status::Active);
        tray.set_snapshot_at(npu_at(0.0), used + HIDE_AFTER - Duration::from_secs(1));
        assert_eq!(tray.status(), ksni::Status::Active);
        tray.set_snapshot_at(npu_at(0.0), used + HIDE_AFTER);
        assert_eq!(tray.status(), ksni::Status::Passive);
    }

    #[test]
    fn low_memory_with_an_app_running_needs_attention() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        tray.set_snapshot_at(npu_at(30.0), tray.npu_used);
        tray.snap.low_memory = true;
        tray.snap.procs.push(resident_app());
        assert_eq!(tray.status(), ksni::Status::NeedsAttention);
    }

    fn tray_with_updates() -> (WatchTray, tokio::sync::mpsc::UnboundedReceiver<Job>) {
        let (jobs, queue) = unbounded_channel();
        (WatchTray::new(None, Some(jobs)), queue)
    }

    #[test]
    fn there_is_no_install_before_a_release_is_found() {
        let (mut tray, mut queue) = tray_with_updates();
        tray.start(INSTALL_UPDATE);
        assert!(queue.try_recv().is_err());
    }

    #[test]
    fn a_second_update_check_waits_for_the_first() {
        let (mut tray, mut queue) = tray_with_updates();
        tray.start(CHECK_UPDATE);
        tray.start(CHECK_UPDATE);
        assert_eq!(queue.try_recv(), Ok(CHECK_UPDATE));
        assert!(queue.try_recv().is_err());
        assert_eq!(tray.update, Update::Checking);
    }

    #[test]
    fn a_driver_check_is_not_held_up_by_an_update_check() {
        let (mut tray, mut queue) = tray_with_updates();
        tray.start(CHECK_UPDATE);
        queue.try_recv().unwrap();
        tray.start(CHECK_DRIVER);
        assert_eq!(queue.try_recv(), Ok(CHECK_DRIVER));
    }

    #[test]
    fn an_available_release_can_be_installed() {
        let (mut tray, mut queue) = tray_with_updates();
        tray.update = Update::Available("v0.5.0".into());
        tray.start(INSTALL_UPDATE);
        assert_eq!(queue.try_recv(), Ok(INSTALL_UPDATE));
        assert_eq!(tray.update, Update::Installing("v0.5.0".into()));
    }

    #[test]
    fn a_stuck_driver_turns_amber_and_stays_in_view() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        tray.driver = Driver::Checked {
            verdict: Verdict::Stuck,
            after_reset: false,
        };
        assert_eq!(tray.state(), State::Warning);
        assert_eq!(tray.status(), ksni::Status::NeedsAttention);
        assert!(tray.tool_tip().description.contains("NPU driver stuck"));
        tray.driver = Driver::Checked {
            verdict: Verdict::Healthy,
            after_reset: true,
        };
        assert_eq!(tray.state(), State::Idle);
        tray.npu_quiet = true;
        assert_eq!(tray.status(), ksni::Status::Passive);
    }

    #[test]
    fn one_driver_job_at_a_time() {
        let (jobs, mut queue) = unbounded_channel();
        let mut tray = WatchTray::new(None, Some(jobs));
        tray.start(CHECK_DRIVER);
        tray.start(RESET_DRIVER);
        assert_eq!(tray.driver, Driver::Checking);
        assert_eq!(queue.try_recv(), Ok(CHECK_DRIVER));
        assert!(queue.try_recv().is_err());
    }

    #[test]
    fn interpreters_are_remembered_after_their_apps_exit() {
        let mut tray = WatchTray::new(None, None);
        tray.set_snapshot(Snapshot {
            pythons: vec!["/bin/sh".into()],
            ..Default::default()
        });
        tray.set_snapshot(Snapshot::default());
        assert_eq!(tray.python(), PathBuf::from("/bin/sh"));
        let tray = WatchTray::new(Some("/opt/py".into()), None);
        assert_eq!(tray.python(), PathBuf::from("/opt/py"));
    }

    #[test]
    fn the_item_names_itself() {
        use ksni::Tray;
        let tray = WatchTray::new(None, None);
        assert_eq!(tray.id(), "vinowatch");
        assert_eq!(tray.title(), "vinoWatch");
        assert_eq!(tray.category(), Category::ApplicationStatus);
        assert_eq!(tray.tool_tip().title, "vinoWatch");
    }

    /// The icon has three looks; the tooltip tells every situation apart, so
    /// nothing depends on seeing its colour, and the menu's first rows say the
    /// same for hosts that show no tooltip.
    #[test]
    fn each_state_has_its_own_text() {
        use ksni::Tray;
        let mut seen: Vec<(&str, String)> = Vec::new();
        for (name, tray) in situations() {
            let tip = tray.tool_tip().description;
            assert!(!tip.trim().is_empty(), "{name} has no tooltip text");
            if let Some((other, _)) = seen.iter().find(|(_, t)| *t == tip) {
                panic!("{name} and {other} both read {tip:?}");
            }
            let labels = all_labels(&tray);
            for line in tip.lines() {
                assert!(
                    labels.iter().any(|l| l == line),
                    "{name}: {line:?} is only in the tooltip"
                );
            }
            seen.push((name, tip));
        }
        let names = State::ALL.map(State::icon_name);
        assert!(names[0] != names[1] && names[1] != names[2] && names[0] != names[2]);
    }

    #[test]
    fn the_amber_icon_has_text_for_each_of_its_warnings() {
        use ksni::Tray;
        let tips: HashMap<_, _> = situations()
            .into_iter()
            .map(|(n, t)| (n, t.tool_tip().description))
            .collect();
        assert!(
            tips["low memory"].contains("(low)"),
            "{}",
            tips["low memory"]
        );
        assert!(tips["stuck"].contains("stuck"), "{}", tips["stuck"]);
    }
}
