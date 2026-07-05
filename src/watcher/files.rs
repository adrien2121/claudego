use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(crate) fn claude_projects_root() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".claude/projects"))
}

pub(crate) fn recent_session_logs(
    projects_root: &Path,
    modified_after: SystemTime,
) -> Vec<(PathBuf, SystemTime)> {
    let mut files = Vec::new();

    if let Ok(project_entries) = fs::read_dir(projects_root) {
        for project_entry in project_entries.flatten() {
            if !project_entry.path().is_dir() {
                continue;
            }

            collect_recent_jsonl_files(project_entry.path(), modified_after, &mut files);
        }
    }

    files
}

pub(crate) fn any_file_changed(
    files: &[(PathBuf, SystemTime)],
    file_size_cache: &HashMap<PathBuf, u64>,
) -> bool {
    files.iter().any(|(path, _)| {
        fs::metadata(path)
            .map(|metadata| match file_size_cache.get(path) {
                Some(cached_size) => *cached_size != metadata.len(),
                None => true,
            })
            .unwrap_or(false)
    })
}

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

            if !metadata.is_file()
                || entry.path().extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }

            if let Ok(modified_time) = metadata.modified() {
                if modified_time > modified_after {
                    files.push((entry.path(), modified_time));
                }
            }
        }
    }
}
