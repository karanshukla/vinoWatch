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

const TRAY_GONE: &str = "the tray icon went away";

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
        let helpers = match helpers {
            0 => String::new(),
            1 => " + 1 helper".into(),
            n => format!(" + {n} helpers"),
        };
        format!(
            "{} · {apps} OpenVINO process{}{helpers}",
            self.snap.npu_label(),
            if apps == 1 { "" } else { "es" }
        )
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
            State::LowMemory
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

    // As vinoWhisper and vinoAuthFace: Plasma files Hardware items in a
    // different part of the panel.
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

    // Hosts hide a Passive item. `the_icon_hides_once_the_npu_has_idled_five_minutes`
    // and `a_stuck_driver_turns_amber_and_stays_in_view` pin when it is one.
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

    // Called when spawn finds no StatusNotifierWatcher, or the host goes away.
    // Keep running: sway and Hyprland start waybar alongside the session, not
    // before it, and exiting here trips the unit's restart limit.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        let why = match reason {
            ksni::OfflineReason::Error(e) => e.to_string(),
            _ => "the tray host went away".into(),
        };
        eprintln!(
            "vinowatch: no tray host ({why}); the icon shows once one starts \
             (GNOME needs the AppIndicator extension)"
        );
        true
    }

    fn watcher_online(&self) {
        eprintln!("vinowatch: a tray host started; the icon is back");
    }

    fn attention_icon_name(&self) -> String {
        if !self.themed_icons {
            return String::new();
        }
        State::LowMemory.icon_name().into()
    }

    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(State::LowMemory)
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "vinoWatch".into(),
            description: [
                Some(self.summary()),
                Some(self.snap.ram_label()),
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
        let mut sampler = Sampler::new(warn_below);
        sampler.poll();
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
    fn the_icon_follows_the_snapshot() {
        let mut tray = WatchTray::new(None, None);
        assert_eq!(tray.state(), State::Idle);
        tray.snap.procs.push(proc_row(1));
        assert_eq!(tray.state(), State::Active);
        // A vino app's helper with no OpenVINO mapped is listed, not active.
        tray.snap.procs[0].openvino = false;
        assert_eq!(tray.state(), State::Idle);
        assert!(
            tray.summary()
                .ends_with("· 0 OpenVINO processes + 1 helper")
        );
        tray.snap.procs[0].openvino = true;
        tray.snap.low_memory = true;
        assert_eq!(tray.state(), State::LowMemory);
        // NPU load from a process we couldn't list still counts.
        tray.snap.low_memory = false;
        tray.snap.procs.clear();
        tray.snap.npu_percent = Some(0.4);
        assert_eq!(tray.state(), State::Idle);
        tray.snap.npu_percent = Some(30.0);
        assert_eq!(tray.state(), State::Active);
        assert_eq!(pixmaps(State::LowMemory).len(), 6);
    }

    #[test]
    fn the_icon_hides_once_the_npu_has_idled_five_minutes() {
        use ksni::Tray;
        let mut tray = WatchTray::new(None, None);
        let start = tray.npu_used;
        let snap = |npu: f64| Snapshot {
            npu_percent: Some(npu),
            ..Default::default()
        };
        // In view from startup until the first five quiet minutes pass.
        tray.set_snapshot_at(snap(0.0), start + Duration::from_secs(60));
        assert_eq!(tray.status(), ksni::Status::Active);
        // Low memory on its own is not about the vino apps.
        tray.snap.low_memory = true;
        assert_eq!(tray.status(), ksni::Status::Active);
        tray.set_snapshot_at(snap(0.4), start + HIDE_AFTER);
        assert_eq!(tray.status(), ksni::Status::Passive);
        // A resident app with the NPU idle doesn't bring it back.
        let app = Proc {
            name: "vino_speak".into(),
            cpu_percent: 3.0,
            ram_bytes: 1,
            npu_open: true,
            ..proc_row(1)
        };
        tray.snap.procs.push(app.clone());
        assert_eq!(tray.status(), ksni::Status::Passive);
        let used = start + HIDE_AFTER * 2;
        tray.set_snapshot_at(snap(30.0), used);
        assert_eq!(tray.status(), ksni::Status::Active);
        tray.set_snapshot_at(snap(0.0), used + HIDE_AFTER - Duration::from_secs(1));
        assert_eq!(tray.status(), ksni::Status::Active);
        tray.snap.low_memory = true;
        tray.snap.procs.push(app);
        assert_eq!(tray.status(), ksni::Status::NeedsAttention);
        tray.set_snapshot_at(snap(0.0), used + HIDE_AFTER);
        assert_eq!(tray.status(), ksni::Status::Passive);
    }

    #[test]
    fn update_jobs_start_one_at_a_time_and_only_for_a_release() {
        let (jobs, mut queue) = unbounded_channel();
        let mut tray = WatchTray::new(None, Some(jobs));
        // Nothing to install yet.
        tray.start(INSTALL_UPDATE);
        assert!(queue.try_recv().is_err());
        tray.start(CHECK_UPDATE);
        tray.start(CHECK_UPDATE);
        assert_eq!(queue.try_recv(), Ok(CHECK_UPDATE));
        assert!(queue.try_recv().is_err(), "a second check while one runs");
        assert_eq!(tray.update, Update::Checking);
        // A driver check isn't held up by it.
        tray.start(CHECK_DRIVER);
        assert_eq!(queue.try_recv(), Ok(CHECK_DRIVER));
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
        assert_eq!(tray.state(), State::LowMemory);
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
        // The amber icon covers two warnings; the text names each.
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
        let names = State::ALL.map(State::icon_name);
        assert!(names[0] != names[1] && names[1] != names[2] && names[0] != names[2]);
    }
}
