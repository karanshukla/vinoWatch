//! The tray icon: the NPU load in the tooltip, one menu row per process.

use std::time::Duration;

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Category, ToolTip, TrayMethods};

use crate::sample::{Sampler, Snapshot};

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
}

impl ksni::Tray for WatchTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "vinoWatch".into()
    }

    fn category(&self) -> Category {
        Category::Hardware
    }

    // A stock theme icon, so there is nothing to install alongside the binary.
    fn icon_name(&self) -> String {
        "utilities-system-monitor".into()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "vinoWatch".into(),
            description: self.summary(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let row = |label: String| -> MenuItem<Self> {
            StandardItem {
                label,
                enabled: false,
                ..Default::default()
            }
            .into()
        };
        let mut items = vec![row(self.summary()), MenuItem::Separator];
        if self.snap.procs.is_empty() {
            items.push(row("No OpenVINO processes".into()));
        }
        items.extend(self.snap.procs.iter().map(|p| row(p.label())));
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

/// Runs until the tray is quit. Errors if no tray host is on the session bus.
pub fn run(interval: Duration) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let mut sampler = Sampler::new();
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
