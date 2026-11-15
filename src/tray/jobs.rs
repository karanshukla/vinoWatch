//! The menu's actions, run off the tray's thread.

use std::collections::HashMap;
use std::path::PathBuf;

use super::WatchTray;
use super::actions::{Driver, DriverJob, Update, UpdateJob};
use crate::driver::{self, Verdict};
use crate::update::{self, Found};

/// Runs an update job and returns where the update row is afterwards.
pub(super) async fn run_update_job(job: UpdateJob, handle: &ksni::Handle<WatchTray>) -> Update {
    match job {
        UpdateJob::Check => {
            let found = tokio::task::spawn_blocking(|| update::check(env!("CARGO_PKG_VERSION")))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            match found {
                Ok(Found::UpToDate) => Update::UpToDate,
                Ok(Found::Newer(tag)) => Update::Available(tag),
                Err(why) => Update::Failed(why),
            }
        }
        UpdateJob::Install => {
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

/// Runs a driver job and returns where the driver is afterwards. A reset is
/// followed by a check.
pub(super) async fn run_driver_job(
    job: DriverJob,
    python: PathBuf,
    handle: &ksni::Handle<WatchTray>,
) -> Driver {
    let after_reset = match job {
        DriverJob::Check => false,
        DriverJob::Reset => {
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
pub(super) async fn notify(driver: &Driver) {
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
