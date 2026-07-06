use super::helpers::DEBOUNCE_DURATION;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Instant;

/// After receiving `first_event`, drain the channel for `DEBOUNCE_DURATION`
/// and return all unique `.jsonl` paths from Modify/Create events.
pub(super) fn debounce_events(
    first_event: notify::Event,
    rx: &Receiver<Result<notify::Event, notify::Error>>,
) -> HashSet<PathBuf> {
    let mut paths = HashSet::new();
    collect_jsonl_paths(&first_event, &mut paths);

    let deadline = Instant::now() + DEBOUNCE_DURATION;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(Ok(event)) => collect_jsonl_paths(&event, &mut paths),
            Ok(Err(_)) => {}                        // notify-internal error, skip
            Err(RecvTimeoutError::Timeout) => break, // debounce window closed
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    paths
}

/// If `event` is a Modify or Create, insert any `.jsonl` paths into `paths`.
fn collect_jsonl_paths(event: &notify::Event, paths: &mut HashSet<PathBuf>) {
    if matches!(
        event.kind,
        notify::EventKind::Modify(_) | notify::EventKind::Create(_)
    ) {
        for path in &event.paths {
            if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                paths.insert(path.clone());
            }
        }
    }
}