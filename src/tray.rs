//! The tray icon: the NPU load in the tooltip, one menu row per process.

use std::sync::OnceLock;
use std::time::Duration;

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Category, ToolTip, TrayMethods};

use crate::icon::{self, State};
use crate::sample::{Sampler, Snapshot};

/// Below this the NPU counts as idle; the busy counter ticks a little even
/// between inferences.
const NPU_BUSY_PERCENT: f64 = 1.0;

struct WatchTray {
    snap: Snapshot,
}

impl WatchTray {
    fn summary(&self) -> String {
        let apps = self.snap.procs.len();
        format!(
            "{} · {apps} OpenVINO process{}",
            self.snap.npu_label(),
            if apps == 1 { "" } else { "es" }
        )
    }

    /// Something is running on OpenVINO: a listed process, or NPU load from
    /// one we couldn't list (another user's, or gone before the poll saw it).
    fn in_use(&self) -> bool {
        !self.snap.procs.is_empty() || self.snap.npu_percent.is_some_and(|p| p >= NPU_BUSY_PERCENT)
    }

    fn state(&self) -> State {
        if self.snap.low_memory {
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
        self.state().icon_name().into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(self.state())
    }

    // Passive, as vinoWhisper does when idle: the host hides the icon (Plasma
    // files it under the expander, waybar drops it) until a vino app runs.
    fn status(&self) -> ksni::Status {
        if !self.in_use() {
            ksni::Status::Passive
        } else if self.snap.low_memory {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }

    fn attention_icon_name(&self) -> String {
        State::LowMemory.icon_name().into()
    }

    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(State::LowMemory)
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "vinoWatch".into(),
            description: format!("{}\n{}", self.summary(), self.snap.ram_label()),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let row = |label: String| -> MenuItem<Self> {
            StandardItem {
                label: menu_text(&label),
                enabled: false,
                ..Default::default()
            }
            .into()
        };
        let mut items = vec![
            row(self.summary()),
            row(self.snap.ram_label()),
            MenuItem::Separator,
        ];
        if self.snap.procs.is_empty() {
            items.push(row("No OpenVINO processes".into()));
        }
        items.extend(self.snap.procs.iter().map(|p| row(p.label())));
        items.push(MenuItem::Separator);
        items.push(row(format!("vinoWatch {}", env!("CARGO_PKG_VERSION"))));
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

/// Runs until the tray is quit. Errors if no tray host is on the session bus.
pub fn run(interval: Duration, warn_below: u8) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let mut sampler = Sampler::new(warn_below);
        sampler.poll();
        let handle = WatchTray {
            snap: Snapshot::default(),
        }
        .spawn()
        .await
        .map_err(|e| format!("couldn't register the tray icon: {e}"))?;
        loop {
            tokio::time::sleep(interval).await;
            let snap = sampler.poll();
            if handle.update(|tray| tray.snap = snap).await.is_none() {
                return Err("the tray icon went away".into());
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_labels_keep_their_underscores() {
        assert_eq!(menu_text("vino_speak pid 1"), "vino__speak pid 1");
        assert_eq!(menu_text("no underscores"), "no underscores");
    }

    #[test]
    fn the_icon_follows_the_snapshot() {
        let mut tray = WatchTray {
            snap: Snapshot::default(),
        };
        assert_eq!(tray.state(), State::Idle);
        tray.snap.procs.push(crate::sample::Proc {
            pid: 1,
            name: "x".into(),
            project: None,
            cpu_percent: 0.0,
            ram_bytes: 0,
            ram_is_pss: true,
            npu_open: false,
        });
        assert_eq!(tray.state(), State::Active);
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
    fn the_icon_hides_while_nothing_uses_openvino() {
        use ksni::Tray;
        let mut tray = WatchTray {
            snap: Snapshot::default(),
        };
        assert_eq!(tray.status(), ksni::Status::Passive);
        // Low memory on its own is not about the vino apps.
        tray.snap.low_memory = true;
        assert_eq!(tray.status(), ksni::Status::Passive);
        tray.snap.npu_percent = Some(0.4);
        assert_eq!(tray.status(), ksni::Status::Passive);
        tray.snap.npu_percent = Some(30.0);
        assert_eq!(tray.status(), ksni::Status::NeedsAttention);
        tray.snap.low_memory = false;
        assert_eq!(tray.status(), ksni::Status::Active);
    }
}
