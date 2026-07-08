use super::helpers::WATCHER_MAX_RETRIES;
use crate::logging::log_to_file;
use notify::{Event, RecommendedWatcher, RecursiveMode, Result, Watcher};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::{self, Receiver};

/// Bundles the OS watcher (must stay alive) with its event receiver.
pub(super) struct WatcherHandle {
    pub(super) _watcher: RecommendedWatcher,
    pub(super) rx: Receiver<Result<Event>>,
}

/// Create a recursive file watcher on `~/.claude/projects`, retrying with
/// exponential backoff (1 s, 2 s, 4 s) on transient failures.
pub(super) fn create_watcher() -> Option<WatcherHandle> {
    let projects_root = crate::watcher::files::claude_projects_root()?;
    let _ = std::fs::create_dir_all(&projects_root);

    for attempt in 0..WATCHER_MAX_RETRIES {
        let (tx, rx) = mpsc::channel(32);
        let event_handler = move |res: Result<Event>| {
            // This closure is the EventHandler. It will be called from a sync context
            // (notify's thread). We use `blocking_send` to send to the async channel.
            // `.ok()` is used to ignore errors if the receiver has been dropped.
            tx.blocking_send(res).ok();
        };
        match notify::recommended_watcher(event_handler) {
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