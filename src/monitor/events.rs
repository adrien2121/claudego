use super::helpers::DEBOUNCE_DURATION;
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

/// After receiving `first_event`, drain the channel for `DEBOUNCE_DURATION`
/// and return all unique `.jsonl` paths from Modify/Create events.
pub(super) async fn debounce_events(
    first_event: notify::Event,
    rx: &mut Receiver<notify::Result<notify::Event>>,
) -> HashSet<PathBuf> {
    let mut paths = HashSet::new();
    collect_jsonl_paths(&first_event, &mut paths);

    // After the first event, keep collecting subsequent events for the debounce duration.
    // The `timeout` will cancel the inner loop when the duration is up.
    let _ = timeout(DEBOUNCE_DURATION, async {
        while let Some(Ok(event)) = rx.recv().await {
            collect_jsonl_paths(&event, &mut paths);
        }
    })
    .await;

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
