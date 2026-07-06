use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Returns the path to the root directory where Claude projects are stored.
pub fn claude_projects_root() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".claude/projects"))
}

/// Finds all `.jsonl` session log files modified after a given time.
pub(crate) fn recent_session_logs(
    projects_root: &Path,
    modified_after: SystemTime,
) -> Vec<(PathBuf, SystemTime)> {
    let mut files = Vec::new();

    if let Ok(project_entries) = fs::read_dir(projects_root) {
        // Iterate through each project directory inside `.claude/projects`.
        for project_entry in project_entries.flatten() {
            if !project_entry.path().is_dir() {
                continue;
            }

            // Collect recent log files from the individual project directory.
            collect_recent_jsonl_files(project_entry.path(), modified_after, &mut files);
        }
    }

    files
}

/// Helper to collect recent `.jsonl` files from a single project directory.
fn collect_recent_jsonl_files(
    project_path: PathBuf,
    modified_after: SystemTime,
    files: &mut Vec<(PathBuf, SystemTime)>,
) {
    if let Ok(entries) = fs::read_dir(project_path) {
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };

            // We only care about `.jsonl` files.
            if !metadata.is_file()
                || entry.path().extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }

            // Check if the file was modified recently enough to be included.
            if let Ok(modified_time) = metadata.modified() {
                if modified_time > modified_after {
                    files.push((entry.path(), modified_time));
                }
            }
        }
    }
}
