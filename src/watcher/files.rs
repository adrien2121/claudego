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
    let mut directories = vec![projects_root.to_path_buf()];

    while let Some(directory) = directories.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                directories.push(entry.path());
                continue;
            }

            if !file_type.is_file()
                || entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("jsonl")
            {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
                continue;
            };
            if modified > modified_after {
                files.push((entry.path(), modified));
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::recent_session_logs;
    use std::fs::{self, File, FileTimes};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "claudego-session-discovery-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn touch(path: &Path, modified: SystemTime) {
        fs::write(path, b"{}\n").unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
    }

    #[test]
    fn finds_direct_and_nested_recent_jsonl_only() {
        let root = TestDir::new();
        let cutoff = SystemTime::now() - Duration::from_secs(60);
        let recent = SystemTime::now();
        let stale = cutoff - Duration::from_secs(1);
        let direct = root.0.join("direct.jsonl");
        let nested_dir = root.0.join("project/deeper");
        fs::create_dir_all(&nested_dir).unwrap();
        let nested = nested_dir.join("nested.jsonl");
        touch(&direct, recent);
        touch(&nested, recent);
        touch(&root.0.join("stale.jsonl"), stale);
        touch(&root.0.join("wrong.txt"), recent);

        let found: Vec<_> = recent_session_logs(&root.0, cutoff)
            .into_iter()
            .map(|(path, _)| path)
            .collect();

        assert_eq!(found.len(), 2);
        assert!(found.contains(&direct));
        assert!(found.contains(&nested));
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let root = TestDir::new();
        let outside = TestDir::new();
        let hidden = outside.0.join("hidden.jsonl");
        touch(&hidden, SystemTime::now());
        symlink(&outside.0, root.0.join("linked-project")).unwrap();

        assert!(recent_session_logs(&root.0, SystemTime::UNIX_EPOCH).is_empty());
    }
}
