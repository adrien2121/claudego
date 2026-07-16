use super::helpers::WATCHER_MAX_RETRIES;
use crate::logging::log_to_file;
use notify::{Event, RecommendedWatcher, RecursiveMode, Result, Watcher};
use std::path::Path;
use tokio::sync::mpsc::{self, Receiver};
use tokio::time::{sleep, Duration};

/// Bundles the OS watcher (must stay alive) with its event receiver.
pub(super) struct WatcherHandle {
    pub(super) _watcher: RecommendedWatcher,
    pub(super) rx: Receiver<Result<Event>>,
}

/// Create a recursive file watcher, retrying with exponential backoff.
pub(super) async fn create_watcher(root: &Path) -> Option<WatcherHandle> {
    if std::fs::create_dir_all(root).is_err() {
        log_to_file("[Watcher Error] Session root could not be prepared.");
        return None;
    }

    for attempt in 0..WATCHER_MAX_RETRIES {
        let (tx, rx) = mpsc::channel(32);
        let event_handler = move |res: Result<Event>| {
            // This closure is the EventHandler. It will be called from a sync context
            // (notify's thread). We use `blocking_send` to send to the async channel.
            // `.ok()` is used to ignore errors if the receiver has been dropped.
            tx.blocking_send(res).ok();
        };
        match notify::recommended_watcher(event_handler) {
            Ok(mut watcher) => match watcher.watch(root, RecursiveMode::Recursive) {
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
                Err(_) => log_to_file(&format!(
                    "[Watcher Error] Session watch failed (attempt {}/{}).",
                    attempt + 1,
                    WATCHER_MAX_RETRIES
                )),
            },
            Err(_) => log_to_file(&format!(
                "[Watcher Error] Watcher initialization failed (attempt {}/{}).",
                attempt + 1,
                WATCHER_MAX_RETRIES
            )),
        }

        if attempt + 1 < WATCHER_MAX_RETRIES {
            let backoff = Duration::from_secs(1 << attempt);
            log_to_file(&format!("[Watcher] Retrying in {:?}…", backoff));
            sleep(backoff).await;
        }
    }

    log_to_file("[Watcher Error] All retries exhausted. Monitor cannot start.");
    None
}
