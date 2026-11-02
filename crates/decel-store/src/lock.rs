// SPDX-License-Identifier: MIT OR Apache-2.0

//! Advisory single-writer lock file (DESIGN §7.1).
//!
//! The lock is an OS file lock on `<file>.lock`, so it is released
//! automatically if the process dies. The file also records the holder's
//! PID and hostname for the "file is open elsewhere" message.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::StoreError;

#[derive(Debug)]
pub(crate) struct LockFile {
    file: File,
    path: PathBuf,
}

pub(crate) fn lock_path(db: &Path) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

pub(crate) fn hostname() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(h) = std::env::var(var)
            && !h.trim().is_empty()
        {
            return h.trim().to_string();
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

impl LockFile {
    pub(crate) fn acquire(db: &Path) -> Result<Self, StoreError> {
        let path = lock_path(db);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let (pid, host) = read_holder(&path);
                return Err(StoreError::Locked { pid, host });
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
        let mut lock = Self { file, path };
        lock.write_holder()?;
        Ok(lock)
    }

    fn write_holder(&mut self) -> std::io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        write!(self.file, "{}\n{}\n", std::process::id(), hostname())?;
        self.file.flush()
    }
}

/// Reads the PID and host of the current holder. On Windows the locked file
/// can't be read by other processes, so both may be unknown.
fn read_holder(path: &Path) -> (Option<u32>, Option<String>) {
    let mut text = String::new();
    if File::open(path)
        .and_then(|mut f| f.read_to_string(&mut text))
        .is_err()
    {
        return (None, None);
    }
    let mut lines = text.lines();
    let pid = lines.next().and_then(|l| l.trim().parse().ok());
    let host = lines
        .next()
        .map(|l| l.trim().to_string())
        .filter(|h| !h.is_empty());
    (pid, host)
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = std::fs::remove_file(&self.path);
    }
}
