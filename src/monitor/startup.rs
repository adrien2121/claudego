use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::monitor::helpers::SCAN_CHUNK_SIZE;
use crate::watcher::files as watcher_files;
use crate::watcher::scan::InitialScanResult;
use chrono::{DateTime, Local};
use memchr;
use std::fs::File;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::SystemTime;

/// Scans a single file by reading it backwards in chunks. This is memory-efficient
/// and robustly finds the most recent rate-limit message by correctly handling
/// log lines that are split across chunk boundaries.
fn scan_file_backwards(path: &PathBuf) -> IoResult<InitialScanResult> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    if file_size == 0 {
        return Ok(InitialScanResult::NoLimitFound);
    }

    let mut buffer = Vec::with_capacity(SCAN_CHUNK_SIZE);
    // `carry_forward` holds a partial line from the start of a chunk, to be
    // prepended to the next chunk read (which is the preceding chunk in the file).
    let mut carry_forward = Vec::new();
    let mut current_pos = file_size;

    while current_pos > 0 {
        let read_start = current_pos.saturating_sub(SCAN_CHUNK_SIZE as u64);
        let read_len = (current_pos - read_start) as usize;

        // Read the next chunk from the file.
        file.seek(SeekFrom::Start(read_start))?;
        buffer.resize(read_len, 0);
        file.read_exact(&mut buffer)?;

        // Prepend the partial line from the previous iteration to complete any split lines.
        buffer.append(&mut carry_forward);

        let content_to_scan: std::borrow::Cow<str>;

        // If we are not at the start of the file, the beginning of our buffer might
        // be a partial line. We save it for the next iteration.
        if read_start > 0 {
            if let Some(first_newline_pos) = memchr::memchr(b'\n', &buffer) {
                carry_forward.extend_from_slice(&buffer[..first_newline_pos]);
                content_to_scan = String::from_utf8_lossy(&buffer[first_newline_pos..]);
            } else {
                // The whole chunk has no newline, so it's all a partial line.
                // We move the buffer's content to carry_forward for the next iteration.
                // `swap` does this efficiently without a new allocation.
                std::mem::swap(&mut carry_forward, &mut buffer);
                current_pos = read_start;
                continue; // Nothing to scan in this iteration.
            }
        } else {
            // This is the first chunk of the file, so process everything.
            content_to_scan = String::from_utf8_lossy(&buffer);
        }

        // We must use `scan_content_for_any_limit` to correctly stop at the
        // first (most recent) limit, even if it's stale.
        match crate::watcher::scan::scan_content_for_any_limit(&content_to_scan) {
            InitialScanResult::Active(limit) => return Ok(InitialScanResult::Active(limit)),
            InitialScanResult::Stale => return Ok(InitialScanResult::Stale),
            InitialScanResult::NoLimitFound => { /* Continue to the previous chunk */ }
        }

        current_pos = read_start;
    }

    Ok(InitialScanResult::NoLimitFound)
}

/// Performs the initial scan of all session files on startup.
pub(super) fn initial_scan(state: &SharedAppState) {
    log_to_file("[Startup] Performing initial rate limit check…");
    let initial_files = watcher_files::claude_projects_root()
        .map(|root| watcher_files::recent_session_logs(&root, SystemTime::UNIX_EPOCH))
        .unwrap_or_default();
    let mut latest_limit: Option<(DateTime<Local>, String)> = None;

    if !initial_files.is_empty() {
        log_to_file(&format!("[Startup] Scanning {} initial session file(s).", initial_files.len()));
    }

    for (path, _) in initial_files {
        // Cache the file size regardless of the scan outcome.
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .file_size_cache
            .insert(path.clone(), file_size);

        match scan_file_backwards(&path) {
            Ok(InitialScanResult::Active(limit)) => {
                if latest_limit.as_ref().map_or(true, |(t, _)| &limit.0 > t) {
                    latest_limit = Some(limit);
                }
            }
            Ok(InitialScanResult::Stale) => {
                // Found a stale limit, which is the most recent one. We can ignore this file.
            }
            Ok(InitialScanResult::NoLimitFound) => {
                // No limit found in the entire file.
            }
            Err(e) => {
                log_to_file(&format!("[Startup] Error scanning file {}: {}", path.display(), e));
            }
        }

        // Add a blank line for readability between file scans during startup.
        log_to_file("");
    }

    if let Some((target_time, time_str)) = latest_limit {
        log_to_file(&format!("[LOCKOUT ON STARTUP] Rate limit found. Target: {}", time_str));
        let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
        app.is_sleeping = true;
        app.lockout.target_time = Some(target_time);
    }
}