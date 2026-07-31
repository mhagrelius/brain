//! Noticing when something other than Brain changes the vault.
//!
//! The vault is a folder of ordinary files, so git, another editor, a sync tool
//! or `mv` can all change it while Brain is open. Ignoring that would make
//! Brain the odd one out among the things editing your notes.
//!
//! `gio::FileMonitor` rather than the `notify` crate: `gio` is already a
//! dependency, and its monitors deliver on the GLib main loop, so there is no
//! worker thread, no channel, and no hand-off back to the UI thread.
//!
//! A `GFileMonitor` on a directory is *not* recursive, so one is created per
//! directory. A vault has tens of directories, not thousands, which is what
//! makes that affordable.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;

/// How long to wait after a change before reporting it.
///
/// A single save from another editor is several events — a temporary file
/// appearing, a rename over the target, an attribute change — and git checking
/// out a branch is hundreds. Coalescing turns a burst into one rescan.
const SETTLE: Duration = Duration::from_millis(400);

/// Watches every directory in a vault and reports that something changed.
pub struct Watcher {
    monitors: Vec<gio::FileMonitor>,
    pending: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Watcher {
    /// Watch `root` and everything under it, calling `on_change` after the
    /// changes stop.
    pub fn new<F>(root: &Path, on_change: F) -> Self
    where
        F: Fn() + 'static,
    {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let on_change = Rc::new(on_change);
        let mut monitors = Vec::new();

        for directory in directories(root) {
            let file = gio::File::for_path(&directory);
            let Ok(monitor) =
                file.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            else {
                // A directory that cannot be watched is not a reason to refuse
                // to open the vault; the reload action still works by hand.
                continue;
            };

            let pending = pending.clone();
            let on_change = on_change.clone();
            monitor.connect_changed(move |_, _, _, event| {
                if !matches!(
                    event,
                    gio::FileMonitorEvent::Changed
                        | gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::Deleted
                        | gio::FileMonitorEvent::Renamed
                        | gio::FileMonitorEvent::MovedIn
                        | gio::FileMonitorEvent::MovedOut
                ) {
                    return;
                }

                // Restart the timer on every event, so a burst reports once
                // when it stops rather than once per file.
                if let Some(source) = pending.borrow_mut().take() {
                    source.remove();
                }
                let pending_inner = pending.clone();
                let on_change = on_change.clone();
                let source = glib::timeout_add_local_once(SETTLE, move || {
                    pending_inner.replace(None);
                    on_change();
                });
                pending.replace(Some(source));
            });

            monitors.push(monitor);
        }

        Self { monitors, pending }
    }

    /// How many directories are being watched. Reported so the caller can say
    /// so, and asserted by tests.
    pub fn watched(&self) -> usize {
        self.monitors.len()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // The timer outlives this object otherwise, and fires a callback that
        // borrows an application which may be shutting down.
        if let Some(source) = self.pending.borrow_mut().take() {
            source.remove();
        }
        for monitor in &self.monitors {
            monitor.cancel();
        }
    }
}

/// Every directory in the vault, the root included.
///
/// Dotted directories are skipped for the same reason the scanner skips them:
/// a vault that is also a git repository would otherwise watch every loose
/// object, and anything Brain itself writes under `.brain/` would come back as
/// an external change and rescan forever.
fn directories(root: &Path) -> Vec<PathBuf> {
    let mut found = vec![root.to_path_buf()];
    let mut queue = vec![root.to_path_buf()];

    while let Some(directory) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let path = entry.path();
                queue.push(path.clone());
                found.push(path);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_directory_is_found_and_dotted_ones_are_not() {
        let root = tempfile::tempdir().expect("temp dir");
        for path in [
            "Meetings",
            "Meetings/2026",
            "attachments",
            ".git",
            ".git/objects",
        ] {
            std::fs::create_dir_all(root.path().join(path)).expect("dir");
        }

        let mut found: Vec<String> = directories(root.path())
            .into_iter()
            .map(|path| {
                path.strip_prefix(root.path())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        found.sort();

        assert_eq!(
            found,
            ["", "Meetings", "Meetings/2026", "attachments"],
            "a vault that is a git repository must not watch its object store"
        );
    }

    #[test]
    fn a_vault_that_is_one_folder_is_one_watch() {
        let root = tempfile::tempdir().expect("temp dir");
        assert_eq!(directories(root.path()).len(), 1);
    }
}
