//! The tray icon: the NPU load in the tooltip, one menu row per process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Category, ToolTip, TrayMethods};

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use vino_ui::family::{self, App};
use vino_ui::tray::MenuIcon;

use crate::driver::{self, Verdict};
use crate::icon::{self, State};
use crate::sample::{Proc, Sampler, Snapshot};
use crate::update::{self, Found};

/// Below this the NPU counts as idle; the busy counter ticks a little even
/// between inferences.
const NPU_BUSY_PERCENT: f64 = 1.0;

/// The icon goes passive once the NPU has been idle this long, as the other
/// vino trays do after their idle minutes.
const HIDE_AFTER: Duration = Duration::from_secs(5 * 60);

/// One process holding this share of RAM gets an amber row: about 3 GiB on a
/// 16 GiB machine. Its RAM meter is full here.
const HEAVY_RAM_SHARE: f64 = 0.2;

/// The menu's actions that run off the tray's thread: the driver probe takes
/// up to a minute, the reload waits for `modprobe`, and the update ones go to
/// the network.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Job {
    Check,
    Reset,
    CheckUpdate,
    /// The release `Update::Available` names.
    InstallUpdate,
}

/// Where "Check for updates" is. Nothing goes online until it is clicked.
#[derive(Clone, Debug, Default, PartialEq)]
enum Update {
    #[default]
    Unchecked,
    Checking,
    UpToDate,
    Available(String),
    Installing(String),
    Failed(String),
}

impl Update {
    fn busy(&self) -> bool {
        matches!(self, Update::Checking | Update::Installing(_))
    }

    fn label(&self) -> String {
        match self {
            Update::Unchecked => "Check for updates".into(),
            Update::Checking => "Checking for updates…".into(),
            Update::UpToDate => "Up to date: check again".into(),
            Update::Available(tag) => format!("Update to {tag}"),
            Update::Installing(tag) => format!("Installing {tag}…"),
            Update::Failed(why) => format!("Update failed ({}): try again", clip(why)),
        }
    }
}

/// Where the stuck-driver check is. Nothing runs until it is asked for.
#[derive(Clone, Debug, Default, PartialEq)]
enum Driver {
    #[default]
    Unchecked,
    Checking,
    Resetting,
    Checked {
        verdict: Verdict,
        after_reset: bool,
    },
    ResetFailed(String),
}

impl Driver {
    fn busy(&self) -> bool {
        matches!(self, Driver::Checking | Driver::Resetting)
    }

    fn stuck(&self) -> bool {
        matches!(
            self,
            Driver::Checked {
                verdict: Verdict::Stuck,
                ..
            }
        )
    }

    fn icon_name(&self) -> &'static str {
        match self {
            Driver::Unchecked | Driver::Checking | Driver::Resetting => MenuIcon::Refresh.name(),
            Driver::Checked {
                verdict: Verdict::Healthy,
                ..
            } => MenuIcon::Ok.name(),
            Driver::Checked {
                verdict: Verdict::Stuck,
                ..
            } => MenuIcon::Warning.name(),
            Driver::Checked {
                verdict: Verdict::Unknown(_),
                ..
            } => MenuIcon::Question.name(),
            Driver::ResetFailed(_) => MenuIcon::Error.name(),
        }
    }

    fn label(&self) -> Option<String> {
        Some(match self {
            Driver::Unchecked => return None,
            Driver::Checking => "Checking the NPU driver…".into(),
            Driver::Resetting => "Reloading the NPU driver…".into(),
            Driver::Checked {
                verdict: Verdict::Healthy,
                after_reset,
            } => if *after_reset {
                "NPU driver reloaded and OK"
            } else {
                "NPU driver OK"
            }
            .into(),
            Driver::Checked {
                verdict: Verdict::Stuck,
                after_reset,
            } => if *after_reset {
                "NPU driver still stuck after a reload: reboot"
            } else {
                "NPU driver stuck: close the apps holding the NPU, then reset it"
            }
            .into(),
            Driver::Checked {
                verdict: Verdict::Unknown(why),
                after_reset,
            } => format!(
                "{}NPU check inconclusive: {}",
                if *after_reset {
                    "NPU driver reloaded. "
                } else {
                    ""
                },
                clip(why)
            ),
            Driver::ResetFailed(why) => format!("NPU reset failed: {}", clip(why)),
        })
    }
}

/// A Python error can name a long library path; a menu row shouldn't.
fn clip(text: &str) -> String {
    const MAX: usize = 100;
    match text.char_indices().nth(MAX) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.into(),
    }
}

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
            Job::Check | Job::Reset => self.driver.busy(),
            Job::CheckUpdate | Job::InstallUpdate => self.update.busy(),
        };
        if busy {
            return;
        }
        if job == Job::InstallUpdate && !matches!(self.update, Update::Available(_)) {
            return;
        }
        if jobs.send(job).is_err() {
            return;
        }
        match job {
            Job::Check => self.driver = Driver::Checking,
            Job::Reset => self.driver = Driver::Resetting,
            Job::CheckUpdate => self.update = Update::Checking,
            Job::InstallUpdate => {
                if let Update::Available(tag) = &self.update {
                    self.update = Update::Installing(tag.clone());
                }
            }
        }
    }

    /// The update row under the version. Without `vinowatch-upgrade` (a build
    /// installed by hand) a newer release opens its page instead.
    fn update_item(&self, can_install: bool) -> MenuItem<Self> {
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
            Update::Available(_) => Box::new(|tray: &mut Self| tray.start(Job::InstallUpdate)),
            _ => Box::new(|tray: &mut Self| tray.start(Job::CheckUpdate)),
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
    fn holders(&self) -> impl Iterator<Item = &crate::sample::Proc> {
        self.snap.procs.iter().filter(|p| p.npu_open)
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
        let used = self.snap.mem.map_or(0.0, |(total, available)| {
            1.0 - available as f64 / total.max(1) as f64
        });
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
            .mem
            .map_or(0.0, |(total, _)| p.ram_bytes as f64 / total.max(1) as f64)
    }

    /// Amber when it's the heavy one (or the biggest while RAM is low), green
    /// while it's working, white while it waits.
    fn proc_state(&self, p: &Proc) -> State {
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

    fn state(&self) -> State {
        // The amber icon covers both warnings.
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

/// The spec lets a tooltip's description carry markup, and XFCE, Cinnamon,
/// MATE, Budgie and Plasma parse it as such: a `<` in a driver error or a
/// process name would otherwise cut the tooltip short.
fn markup_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Tray hosts that draw a themed `-symbolic` icon as the SVG has it instead of
/// in the panel's text colour, by the watcher's process name (`comm`, cut to
/// 15 characters). The glyph is near-black, and these panels are usually dark.
const NO_RECOLOUR_HOSTS: &[&str] = &[
    "lxqt-panel",
    "mate-panel",
    // mate-panel's notification area when it runs out of process.
    "notification-ar",
    "swaybar",
];

/// Whether the tray host recolours `-symbolic` icons. `watcher` is the process
/// name owning `org.kde.StatusNotifierWatcher`, `desktop` is
/// `XDG_CURRENT_DESKTOP`; the desktop covers a watcher we couldn't name.
fn host_recolours(watcher: Option<&str>, desktop: Option<&str>) -> bool {
    if watcher.is_some_and(|name| NO_RECOLOUR_HOSTS.contains(&name.trim())) {
        return false;
    }
    let desktop = desktop.unwrap_or_default();
    watcher.is_some() || !desktop.split(':').any(|d| d == "LXQt" || d == "MATE")
}

/// The process name of whoever owns the tray watcher now, with its unique bus
/// name so a change of host is noticed.
async fn watcher_host(dbus: &zbus::fdo::DBusProxy<'_>) -> Option<(String, Option<String>)> {
    let name = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher").ok()?;
    let owner = dbus.get_name_owner(name).await.ok()?;
    let pid = dbus
        .get_connection_unix_process_id(owner.as_ref().into())
        .await
        .ok();
    let comm = pid.and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok());
    Some((owner.to_string(), comm))
}

/// ksni treats "_" as an access-key marker and hides it; a doubled one is a
/// literal underscore. Process names like `vino_speak` have them.
fn menu_text(label: &str) -> String {
    label.replace('_', "__")
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

    // Passive, as vinoWhisper does when idle: the host hides the icon (Plasma
    // files it under the expander, waybar drops it) until the NPU is used.
    fn status(&self) -> ksni::Status {
        // A stuck driver stays in view after its apps exit: that is when the
        // reset can run.
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
        let mut items = vec![
            note(self.summary(), self.npu_icon()),
            note(self.snap.ram_label(), self.ram_icon()),
        ];
        items.extend(
            self.driver
                .label()
                .map(|label| note(label, RowIcon::Named(self.driver.icon_name()))),
        );
        items.push(MenuItem::Separator);
        if self.snap.procs.is_empty() {
            let empty = RowIcon::Drawn(icon::meter_png(State::Idle, &[0.0; 3]));
            items.push(note("No OpenVINO processes".into(), empty));
        }
        items.extend(
            self.snap
                .procs
                .iter()
                .map(|p| note(p.label(), self.proc_icon(p))),
        );
        items.push(MenuItem::Separator);
        // Once checked, keep these: a reload whose modprobe failed leaves no
        // device, and the reset is how to retry.
        if self.snap.npu_present || self.driver != Driver::Unchecked {
            items.extend(self.driver_items(driver::reset_installed()));
            items.push(MenuItem::Separator);
        }
        items.push(note(
            format!("vinoWatch {}", env!("CARGO_PKG_VERSION")),
            RowIcon::Named("vinowatch"),
        ));
        items.push(self.update_item(update::upgrade_command().is_some()));
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: MenuIcon::Quit.name().into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

impl WatchTray {
    /// `reset_installed`: whether `install-npu-reset.sh` has been run.
    fn driver_items(&self, reset_installed: bool) -> Vec<MenuItem<Self>> {
        let busy = self.driver.busy();
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: "Check NPU driver".into(),
                icon_name: MenuIcon::Search.name().into(),
                enabled: !busy,
                activate: Box::new(|tray: &mut Self| tray.start(Job::Check)),
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
                // Nothing is stopped for you; the reset waits for them.
                enabled: !busy && holders.is_empty(),
                activate: Box::new(|tray: &mut Self| tray.start(Job::Reset)),
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

/// Runs an update job and returns where the update row is afterwards.
async fn run_update(job: Job, handle: &ksni::Handle<WatchTray>) -> Update {
    match job {
        Job::CheckUpdate => {
            let found = tokio::task::spawn_blocking(|| update::check(env!("CARGO_PKG_VERSION")))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            match found {
                Ok(Found::UpToDate) => Update::UpToDate,
                Ok(Found::Newer(tag)) => Update::Available(tag),
                Err(why) => Update::Failed(why),
            }
        }
        _ => {
            let tag = handle
                .update(|tray| match &tray.update {
                    Update::Installing(tag) => Some(tag.clone()),
                    _ => None,
                })
                .await
                .flatten();
            let (Some(tag), Some(upgrade)) = (tag, update::upgrade_command()) else {
                return Update::Failed("no vinowatch-upgrade".into());
            };
            let failed_tag = tag.clone();
            // Success restarts the tray, which ends this process on the new
            // binary; returning at all means it didn't happen.
            match tokio::task::spawn_blocking(move || update::install(&upgrade, &tag)).await {
                Ok(Err(why)) => Update::Failed(why),
                Ok(Ok(())) => Update::Available(failed_tag),
                Err(e) => Update::Failed(e.to_string()),
            }
        }
    }
}

/// Runs one job and returns where the driver is afterwards.
async fn run_job(job: Job, python: PathBuf, handle: &ksni::Handle<WatchTray>) -> Driver {
    let after_reset = match job {
        Job::Check | Job::CheckUpdate | Job::InstallUpdate => false,
        Job::Reset => {
            let reset = tokio::task::spawn_blocking(driver::reset)
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            if let Err(why) = reset {
                return Driver::ResetFailed(why);
            }
            handle.update(|t| t.driver = Driver::Checking).await;
            true
        }
    };
    let verdict = tokio::task::spawn_blocking(move || driver::probe(&python))
        .await
        .unwrap_or_else(|e| Verdict::Unknown(e.to_string()));
    Driver::Checked {
        verdict,
        after_reset,
    }
}

/// The menu closes on click, so without this a check that ends "OK" looks
/// like it did nothing. Best effort: no notification server, no popup.
async fn notify(driver: &Driver) {
    let Some(summary) = driver.label() else {
        return;
    };
    let Ok(bus) = zbus::Connection::session().await else {
        return;
    };
    let hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    let _ = bus
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "vinoWatch",
                0u32,
                "vinowatch",
                summary,
                "",
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await;
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
        // The unique name of the watcher's owner last time; None until a tray
        // host is up. Asked every poll: a host can be replaced at any time.
        let mut host = None;
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticks.tick() => {
                    if let Some(dbus) = &dbus {
                        let now = watcher_host(dbus).await;
                        if now.as_ref().map(|(owner, _)| owner) != host.as_ref() {
                            let comm = now.as_ref().and_then(|(_, comm)| comm.as_deref());
                            let themed = host_recolours(comm, desktop.as_deref());
                            handle.update(|tray| tray.themed_icons = themed).await;
                            host = now.map(|(owner, _)| owner);
                        }
                    }
                    let snap = sampler.poll();
                    if handle.update(|tray| tray.set_snapshot(snap)).await.is_none() {
                        return Err("the tray icon went away".into());
                    }
                }
                Some(job) = queue.recv() => {
                    if matches!(job, Job::CheckUpdate | Job::InstallUpdate) {
                        let handle = handle.clone();
                        tokio::spawn(async move {
                            let update = run_update(job, &handle).await;
                            handle.update(|tray| tray.update = update).await;
                        });
                        continue;
                    }
                    let Some(python) = handle.update(|tray| tray.python()).await else {
                        return Err("the tray icon went away".into());
                    };
                    let handle = handle.clone();
                    tokio::spawn(async move {
                        let driver = run_job(job, python, &handle).await;
                        notify(&driver).await;
                        handle.update(|tray| tray.driver = driver).await;
                    });
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn tooltip_text_is_escaped() {
        assert_eq!(markup_escape("a <b> & c"), "a &lt;b&gt; &amp; c");
    }

    #[test]
    fn hosts_that_dont_recolour_get_pixmaps() {
        assert!(host_recolours(Some("waybar\n"), None));
        assert!(host_recolours(Some("plasmashell"), Some("KDE")));
        assert!(!host_recolours(Some("swaybar\n"), Some("sway")));
        assert!(!host_recolours(Some("lxqt-panel"), None));
        assert!(!host_recolours(Some("notification-ar"), None));
        // No name for the watcher: go by the desktop.
        assert!(!host_recolours(None, Some("X-Ubuntu:MATE")));
        assert!(host_recolours(None, Some("XFCE")));
        assert!(host_recolours(None, None));
        let mut tray = WatchTray::new(None, None);
        tray.themed_icons = false;
        assert_eq!(ksni::Tray::icon_name(&tray), "");
        assert_eq!(ksni::Tray::attention_icon_name(&tray), "");
        assert!(!ksni::Tray::icon_pixmap(&tray).is_empty());
    }

    #[test]
    fn menu_labels_keep_their_underscores() {
        assert_eq!(menu_text("vino_speak pid 1"), "vino__speak pid 1");
        assert_eq!(menu_text("no underscores"), "no underscores");
    }

    #[test]
    fn the_icon_follows_the_snapshot() {
        let mut tray = WatchTray::new(None, None);
        assert_eq!(tray.state(), State::Idle);
        tray.snap.procs.push(crate::sample::Proc {
            pid: 1,
            name: "x".into(),
            project: None,
            cpu_percent: 0.0,
            ram_bytes: 0,
            ram_is_pss: true,
            npu_open: false,
            openvino: true,
        });
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
    fn a_process_row_is_coloured_by_what_it_is_doing() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let proc = |pid, cpu_percent, ram_bytes, npu_open| Proc {
            pid,
            name: "x".into(),
            project: None,
            cpu_percent,
            ram_bytes,
            ram_is_pss: true,
            npu_open,
            openvino: true,
        };
        let mut tray = WatchTray::new(None, None);
        tray.snap.mem = Some((16 * GIB, 8 * GIB));
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
            pid: 1,
            name: "x".into(),
            project: None,
            cpu_percent: 0.0,
            ram_bytes: 0,
            ram_is_pss: true,
            npu_open: true,
            openvino: true,
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
            pid: 1,
            name: "vino_speak".into(),
            project: None,
            cpu_percent: 3.0,
            ram_bytes: 1,
            ram_is_pss: true,
            npu_open: true,
            openvino: true,
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

    fn labels(tray: &WatchTray) -> Vec<String> {
        use ksni::Tray;
        texts(tray.menu())
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
    fn update_jobs_start_one_at_a_time_and_only_for_a_release() {
        let (jobs, mut queue) = unbounded_channel();
        let mut tray = WatchTray::new(None, Some(jobs));
        // Nothing to install yet.
        tray.start(Job::InstallUpdate);
        assert!(queue.try_recv().is_err());
        tray.start(Job::CheckUpdate);
        tray.start(Job::CheckUpdate);
        assert_eq!(queue.try_recv(), Ok(Job::CheckUpdate));
        assert!(queue.try_recv().is_err(), "a second check while one runs");
        assert_eq!(tray.update, Update::Checking);
        // A driver check isn't held up by it.
        tray.start(Job::Check);
        assert_eq!(queue.try_recv(), Ok(Job::Check));
        tray.update = Update::Available("v0.5.0".into());
        tray.start(Job::InstallUpdate);
        assert_eq!(queue.try_recv(), Ok(Job::InstallUpdate));
        assert_eq!(tray.update, Update::Installing("v0.5.0".into()));
    }

    fn texts(items: Vec<MenuItem<WatchTray>>) -> Vec<String> {
        items
            .into_iter()
            .filter_map(|item| match item {
                MenuItem::Standard(item) => Some(item.label),
                _ => None,
            })
            .collect()
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
        tray.snap.procs.push(crate::sample::Proc {
            pid: 7,
            name: "vino_speak".into(),
            project: None,
            cpu_percent: 0.0,
            ram_bytes: 0,
            ram_is_pss: true,
            npu_open: true,
            openvino: true,
        });
        assert_eq!(enabled(&tray), Some(false));
        let items = texts(tray.driver_items(true));
        assert_eq!(items[2], "Close these first:");
        assert!(items[3].starts_with("vino__speak pid 7"), "{}", items[3]);
        tray.driver = Driver::Resetting;
        assert_eq!(enabled(&tray), Some(false));
    }

    #[test]
    fn long_errors_are_clipped() {
        let tray = WatchTray {
            driver: Driver::Checked {
                verdict: Verdict::Unknown("x".repeat(500)),
                after_reset: true,
            },
            ..WatchTray::new(None, None)
        };
        let label = tray.driver.label().unwrap();
        assert!(label.starts_with("NPU driver reloaded. NPU check inconclusive: xxx"));
        assert!(label.ends_with("x…"));
        assert!(label.chars().count() < 200);
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
        tray.start(Job::Check);
        tray.start(Job::Reset);
        assert_eq!(tray.driver, Driver::Checking);
        assert_eq!(queue.try_recv(), Ok(Job::Check));
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

    fn app(pid: u32, name: &str, npu_open: bool) -> Proc {
        Proc {
            pid,
            name: name.into(),
            project: None,
            cpu_percent: 12.0,
            ram_bytes: 512 * 1024 * 1024,
            ram_is_pss: true,
            npu_open,
            openvino: true,
        }
    }

    /// Every situation that changes the icon, tooltip or menu.
    fn situations() -> Vec<(&'static str, WatchTray)> {
        const GIB: u64 = 1024 * 1024 * 1024;
        let base = || {
            let mut tray = WatchTray::new(None, None);
            tray.snap.mem = Some((16 * GIB, 8 * GIB));
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
        tray.snap.mem = Some((16 * GIB, GIB / 2));
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
    fn spoken(label: &str) -> String {
        label
            .replace("__", "\u{0}")
            .replace('_', "")
            .replace('\u{0}', "_")
    }

    /// One item: labelled, shown, and with any underscore kept.
    fn check(label: &str, visible: bool, out: &mut Vec<String>) {
        assert!(!spoken(label).trim().is_empty(), "unlabelled item");
        assert!(visible, "{label:?} is hidden instead of disabled");
        assert!(
            !label.replace("__", "").contains('_'),
            "{label:?} loses an underscore"
        );
        out.push(spoken(label));
    }

    /// Every label in a menu, depth first, checking each item on the way.
    fn walk(items: &[MenuItem<WatchTray>], out: &mut Vec<String>) {
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

    fn all_labels(tray: &WatchTray) -> Vec<String> {
        use ksni::Tray;
        let mut out = Vec::new();
        walk(&tray.menu(), &mut out);
        for reset_installed in [false, true] {
            walk(&tray.driver_items(reset_installed), &mut out);
        }
        out
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
}
