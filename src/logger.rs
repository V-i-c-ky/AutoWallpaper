use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;

/// Simple file-based logger with timestamp formatting.
pub struct Logger {
    path: PathBuf,
    initialized: bool,
}

impl Logger {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            initialized: false,
        }
    }

    /// Write a timestamped message to the log file.
    /// On first call, adds a blank line separator if the file already has content.
    pub fn log(&mut self, message: &str) {
        if !self.initialized {
            if fs::metadata(&self.path).is_ok_and(|m| m.len() > 0) {
                if let Ok(mut f) = OpenOptions::new().append(true).open(&self.path) {
                    let _ = writeln!(f);
                }
            }
            self.initialized = true;
        }

        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(f, "[{ts}] {message}");
        }
    }
}
