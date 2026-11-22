//! The menu's actions, run off the tray's thread.

use std::collections::HashMap;
use std::path::PathBuf;

use super::WatchTray;
use super::actions::{Driver, DriverJob, Job, Update, UpdateJob};
use crate::driver::{self, Verdict};
use crate::update::{self, Found};

/// The tray item is gone, so there is nothing left to run jobs for.
pub(super) struct TrayGone;

/// Starts `job` on a task of its own and writes where it ended back to the
/// tray. `WatchTray::start` has already marked it as running.
pub(super) async fn dispatch(job: Job, handle: &ksni::Handle<WatchTray>) -> Result<(), TrayGone> {
    let handle = handle.clone();
    match job {
        Job::Update(job) => {
            tokio::spawn(async move {
                let update = run_update_job(job, &handle).await;
                handle.update(|tray| tray.update = update).await;
            });
        }
        Job::Driver(job) => {
            let python = handle.update(|tray| tray.python()).await.ok_or(TrayGone)?;
            tokio::spawn(async move {
                let driver = run_driver_job(job, python, &handle).await;
                notify(&driver).await;
                handle.update(|tray| tray.driver = driver).await;
            });
        }
    }
    Ok(())
}

/// Runs an update job and returns where the update row is afterwards.
async fn run_update_job(job: UpdateJob, handle: &ksni::Handle<WatchTray>) -> Update {
    match job {
        UpdateJob::Check => {
            let found = tokio::task::spawn_blocking(|| update::check(crate::version::VERSION))
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
                return Update::Failed("vinowatch-upgrade not found".into());
            };
            let failed_tag = tag.clone();
            // Success restarts the tray and ends this process; returning means it didn't.
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
async fn run_driver_job(
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
