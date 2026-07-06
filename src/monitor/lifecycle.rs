use super::helpers::WATCHER_MAX_RETRIES;
use crate::logging::log_to_file;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::Duration;

/// Bundles the OS watcher (must stay alive) with its event receiver.
pub(super) struct WatcherHandle {
    pub(super) _watcher: RecommendedWatcher,
    pub(super) rx: Receiver<Result<notify::Event, notify::Error>>,
}

/// Create a recursive file watcher on `~/.claude/projects`, retrying with
/// exponential backoff (1 s, 2 s, 4 s) on transient failures.
pub(super) fn create_watcher() -> Option<WatcherHandle> {
    let projects_root = crate::watcher::files::claude_projects_root()?;
    let _ = std::fs::create_dir_all(&projects_root);

    for attempt in 0..WATCHER_MAX_RETRIES {
        let (tx, rx) = channel();
        match notify::recommended_watcher(tx) {
            Ok(mut watcher) => match watcher.watch(&projects_root, RecursiveMode::Recursive) {
                Ok(()) => {
                    if attempt > 0 {
                        log_to_file(&format!(
                            "[Watcher] Initialized on attempt {}.",
                            attempt + 1
                        ));
                    }
                    return Some(WatcherHandle {
                        _watcher: watcher,
                        rx,
                    });
                }
                Err(e) => log_to_file(&format!(
                    "[Watcher Error] watch() failed (attempt {}/{}): {}",
                    attempt + 1,
                    WATCHER_MAX_RETRIES,
                    e
                )),
            },
            Err(e) => log_to_file(&format!(
                "[Watcher Error] recommended_watcher() failed (attempt {}/{}): {}",
                attempt + 1,
                WATCHER_MAX_RETRIES,
                e
            )),
        }

        if attempt + 1 < WATCHER_MAX_RETRIES {
            let backoff = Duration::from_secs(1 << attempt);
            log_to_file(&format!("[Watcher] Retrying in {:?}…", backoff));
            thread::sleep(backoff);
        }
    }

    log_to_file("[Watcher Error] All retries exhausted. Monitor cannot start.");
    None
}