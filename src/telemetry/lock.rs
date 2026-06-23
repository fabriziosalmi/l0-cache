//! Advisory file locking for telemetry sidecar files.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;

/// Advisory file lock. On unix this is `flock(2)` on a sidecar lock file:
/// kernel-released on process death (no staleness heuristic needed, unlike
/// the old mkdir-based lock whose 10s mtime-break could steal a live lock and
/// whose rotation window could drop a concurrent append). Non-unix keeps the
/// mkdir fallback. Locking stays best-effort at the call sites: telemetry
/// must never block or fail the wrapped command.
pub(crate) struct FileLock {
    path: PathBuf,
    #[cfg(unix)]
    file: Option<fs::File>,
    #[cfg(not(unix))]
    acquired: bool,
}

impl FileLock {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            #[cfg(unix)]
            file: None,
            #[cfg(not(unix))]
            acquired: false,
        }
    }

    /// Lock handle guarding `data_file`. The lock path is protocol-specific:
    /// flock uses `<file>.flock` (a regular file), deliberately DISTINCT from
    /// the legacy mkdir protocol's `<file>.lock` directory — so during a
    /// mixed-version window (agent shells still running a pre-flock binary
    /// across an upgrade) the old binary's mkdir lock keeps working at its
    /// own path instead of stalling 10×50ms per run on a path whose
    /// filesystem type changed under it.
    pub(crate) fn for_data_file(data_file: &std::path::Path) -> Self {
        #[cfg(unix)]
        let lock_path = data_file.with_extension("jsonl.flock");
        #[cfg(not(unix))]
        let lock_path = data_file.with_extension("jsonl.lock");
        FileLock::new(lock_path)
    }

    #[cfg(unix)]
    pub(crate) fn lock(&mut self) -> bool {
        use std::os::unix::io::AsRawFd;
        let file = match OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&self.path)
        {
            Ok(f) => f,
            Err(_) => return false,
        };
        for _ in 0..10 {
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                self.file = Some(file);
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    /// Whether this handle currently holds the lock (test observability).
    #[cfg(test)]
    pub(crate) fn acquired(&self) -> bool {
        #[cfg(unix)]
        {
            self.file.is_some()
        }
        #[cfg(not(unix))]
        {
            self.acquired
        }
    }

    #[cfg(not(unix))]
    pub(crate) fn lock(&mut self) -> bool {
        for _ in 0..10 {
            match fs::create_dir(&self.path) {
                Ok(_) => {
                    self.acquired = true;
                    return true;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Ok(meta) = fs::metadata(&self.path) {
                        if let Ok(modified) = meta.modified() {
                            if let Ok(elapsed) = modified.elapsed() {
                                if elapsed.as_secs() > 10 {
                                    let _ = fs::remove_dir(&self.path);
                                }
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => return false,
            }
        }
        false
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // Closing the fd releases the flock; the lock file itself stays
            // (unlinking it would open a two-owners race with a third process
            // creating a fresh file at the same path).
            self.file.take();
        }
        #[cfg(not(unix))]
        if self.acquired {
            let _ = fs::remove_dir(&self.path);
        }
    }
}
