use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggerPaths {
    pub log: PathBuf,
    pub port: PathBuf,
}

impl LoggerPaths {
    pub fn for_pid(pid: u32) -> Self {
        Self::for_pid_in(&std::env::temp_dir(), pid)
    }

    pub fn for_pid_in(temp_dir: &Path, pid: u32) -> Self {
        Self {
            log: temp_dir.join(format!("botsitter-{pid}.log")),
            port: temp_dir.join(format!("botsitter-{pid}.port")),
        }
    }
}

pub fn current_logger_paths() -> LoggerPaths {
    LoggerPaths::for_pid(std::process::id())
}

pub fn pid_from_port_path(path: &Path) -> Option<u32> {
    path.file_name()?
        .to_str()?
        .strip_prefix("botsitter-")?
        .strip_suffix(".port")?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::{pid_from_port_path, LoggerPaths};
    use std::path::Path;

    #[test]
    fn logger_paths_are_pid_scoped() {
        let paths = LoggerPaths::for_pid_in(Path::new("/tmp/test"), 42);
        assert_eq!(paths.log, Path::new("/tmp/test/botsitter-42.log"));
        assert_eq!(paths.port, Path::new("/tmp/test/botsitter-42.port"));
        assert_eq!(pid_from_port_path(&paths.port), Some(42));
        assert_eq!(
            pid_from_port_path(Path::new("/tmp/test/botsitter.port")),
            None
        );
        assert_eq!(
            pid_from_port_path(Path::new("/tmp/test/botsitter-x.port")),
            None
        );
        assert_eq!(
            pid_from_port_path(Path::new("/tmp/test/claudego-42.port")),
            None
        );
    }
}
