// ABOUTME: Cross-process write lock shared with code-review-graph (.code-review-graph/write.lock).
// ABOUTME: Exclusive flock; the holder's PID is written into the file and cleared on release.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

pub const WRITE_LOCK_NAME: &str = "write.lock";

/// Held for a whole build. The kernel releases it if the process dies.
pub struct WriteLock {
    file: File,
}

impl WriteLock {
    /// `Ok(None)` when another process holds the lock and `blocking` is false.
    pub fn acquire(project_root: &Path, blocking: bool) -> Result<Option<Self>> {
        let dir = project_root.join(".code-review-graph");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(WRITE_LOCK_NAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        if blocking {
            file.lock()?;
        } else {
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(err)) => return Err(err.into()),
            }
        }
        let mut lock = Self { file };
        lock.record_holder();
        Ok(Some(lock))
    }

    /// Best effort, as in CRG: the PID is a diagnostic, not part of the lock.
    fn record_holder(&mut self) {
        let _ = self.file.set_len(0);
        let _ = self.file.seek(SeekFrom::Start(0));
        let _ = writeln!(self.file, "{}", std::process::id());
        let _ = self.file.flush();
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        let _ = self.file.unlock();
    }
}
