use crate::logging::{log_to_file, log_with_content};
use crate::models::SharedAppState;
use crate::monitor::helpers::{DEFER_SCAN_INTERVAL, PTY_BUSY_THRESHOLD, SCAN_CHUNK_SIZE};
use crate::pty_bridge::SharedPtyWriter;
use chrono::{DateTime, Local};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// To prevent unbounded memory usage when a log file has a very large amount of new
/// content, we cap the content collected for logging to 1 MiB.
const MAX_PREVIEW_CONTENT_SIZE: usize = 1_048_576;

/// Scans a set of changed paths and updates the application state.
pub(super) fn scan_and_update_state(
    paths: HashSet<PathBuf>,
    state: &SharedAppState,
    next_log_time: &mut Instant,
) {
    // --- Busy-Wait Logic ---
    // Before scanning, check if the PTY is actively streaming. If so, wait.
    // This prevents the file scan's I/O from competing with the PTY reader thread,
    // which is a direct cause of the "stalled stream" error.
    let activity_tracker = state.lock().unwrap().last_pty_activity.clone();
    loop {
        let last_activity_ns = activity_tracker.load(Ordering::Relaxed);
        let pty_is_busy = if last_activity_ns == 0 {
            // If the timestamp is 0, it's uninitialized. Assume not busy.
            false
        } else {
            // Check if the duration since the last activity is within our threshold.
            let now_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;
            let elapsed_ns = now_ns.saturating_sub(last_activity_ns);
            Duration::from_nanos(elapsed_ns) < PTY_BUSY_THRESHOLD
        };

        if pty_is_busy {
            log_to_file(&format!(
                "[Scan Postponed] Command is actively streaming output. Deferring file scan for {:?}.",
                DEFER_SCAN_INTERVAL
            ));
            std::thread::sleep(DEFER_SCAN_INTERVAL);
        } else {
            break; // PTY is quiet, proceed with the scan.
        }
    }

    // Log which files are being scanned.
    let path_names: Vec<_> = paths
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    log_to_file(&format!(
        "[File Event] Triggering scan. Changed files:\n{}",
        path_names.join("\n")
    ));

    for path in paths {
        let old_size = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .file_size_cache
            .get(&path)
            .copied()
            .unwrap_or(0);

        let mtime_before = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());

        let (new_content, bytes_read, limit_opt) = match scan_new_content_chunked(&path, old_size) {
            Ok(result) => result,
            Err(_) => continue,
        };

        if bytes_read == 0 {
            continue;
        }

        let mtime_after = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());

        // If the file was modified during our read, we abort processing for this cycle.
        // The file watcher has already picked up the new change, which will trigger a
        // new scan after the debounce period. This ensures we only act on stable data.
        if mtime_before.is_some() && mtime_before != mtime_after {
            log_to_file(&format!(
                "[Scan Aborted] Concurrent modification of {}. Rescheduling.",
                path.display()
            ));
            continue;
        }

        let new_size = old_size + bytes_read;
        // Generate a preview of the new content for the logs.
        let content_preview = crate::monitor::formatters::create_content_preview(&new_content);
        log_with_content(&format!("[File Content] New data in {}:", path.display()), content_preview);

        let mut app = state.lock().unwrap_or_else(|e| e.into_inner());

        app.file_size_cache.insert(path.clone(), new_size);
        if let Some((target_time, time_str)) = limit_opt {
            log_to_file(&format!("[LOCKOUT DETECTED] Rate limit hit! Target: {}", time_str));
            app.is_sleeping = true;
            app.lockout.target_time = Some(target_time);
            *next_log_time = Instant::now();
        }
    }
}

/// Scans new content in a file from a given offset, chunk by chunk, to avoid
/// allocating a potentially huge string for all the new content at once.
/// Returns all new content read (for logging), the total bytes read, and the
/// most recent active limit found.
fn scan_new_content_chunked(
    path: &PathBuf,
    old_size: u64,
) -> IoResult<(String, u64, Option<(DateTime<Local>, String)>)> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(old_size))?;

    let mut reader = std::io::BufReader::new(file);
    let mut full_new_content = String::new();
    let mut latest_limit: Option<(DateTime<Local>, String)> = None;
    let mut total_bytes_read = 0;

    let mut chunk_buf = vec![0; SCAN_CHUNK_SIZE];
    loop {
        let bytes_read = reader.read(&mut chunk_buf)?;

        if bytes_read == 0 {
            break;
        }
        total_bytes_read += bytes_read as u64;

        let data_slice = &chunk_buf[..bytes_read];

        // Using from_utf8_lossy is pragmatic for logging.
        // We accumulate the new content for logging, but cap it to avoid
        // unbounded memory allocation if the file change is massive.
        let content_chunk = String::from_utf8_lossy(data_slice);
        if full_new_content.len() < MAX_PREVIEW_CONTENT_SIZE {
            let remaining_space = MAX_PREVIEW_CONTENT_SIZE - full_new_content.len();
            if content_chunk.len() <= remaining_space {
                full_new_content.push_str(&content_chunk);
            } else {
                full_new_content.push_str(&content_chunk[..remaining_space]);
            }
        }

        // Scan this chunk for a limit. Since we are reading forwards, any limit
        // found in a later chunk is newer than one from a previous chunk.
        if let Some(limit) = crate::watcher::scan::scan_content_for_limit(&content_chunk) {
            latest_limit = Some(limit);
        }
    }

    Ok((full_new_content, total_bytes_read, latest_limit))
}

/// Handles the logic for when a lockout expires.
pub(super) fn handle_expiry(state: &SharedAppState, writer: &SharedPtyWriter, expired_target: DateTime<Local>) {
    log_to_file("[Trigger] Reset time reached. Injecting 'continue' command…");
    {
        let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = w.write_all(b"continue\r");
        let _ = w.flush();
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    // Atomically check and set: only clear the lockout if it's the one we just handled.
    // This prevents a race condition where a new lockout is detected right as an old one expires.
    if s.lockout.target_time == Some(expired_target) {
        s.is_sleeping = false;
        s.lockout.target_time = None;
        s.file_size_cache.clear();
        log_to_file("[System] Resuming passive file monitoring.");
    } else {
        // Another lockout was set in the meantime. Don't clear it.
        log_to_file("[System] Expiry handled, but a newer lockout has already been detected. State not cleared.");
    }
}