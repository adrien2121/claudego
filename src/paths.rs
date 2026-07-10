use std::path::PathBuf;

/// Returns the path to the directory used for temporary files.
fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// Returns the path to the main log file.
pub fn log_path() -> PathBuf {
    temp_dir().join("claudego.log")
}

/// Returns the path to the file that stores the port number for the live log server.
pub fn port_path() -> PathBuf {
    temp_dir().join("claudego.port")
}
