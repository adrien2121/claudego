use std::env;
use std::path::PathBuf;
use std::process::Command;

fn log_path() -> PathBuf {
    env::temp_dir().join("claudego.log")
}

fn main() {
    let path = log_path();
    
    if !path.exists() {
        println!("No log file found at {}.", path.display());
        println!("Run `claudego -l -- claude` first to generate logs.");
        return;
    }
    
    println!("Tailing logs from {}...", path.display());
    println!("(Press Ctrl+C to stop)");
    
    #[cfg(unix)]
    {
        let mut child = Command::new("tail")
            .arg("-f")
            .arg(&path)
            .spawn()
            .expect("Failed to execute tail command");
            
        let _ = child.wait();
    }
    
    #[cfg(windows)]
    {
        let mut child = Command::new("powershell")
            .arg("-Command")
            .arg(format!("Get-Content '{}' -Wait", path.display()))
            .spawn()
            .expect("Failed to execute PowerShell command");
            
        let _ = child.wait();
    }
}
