//! Exclusive file locks released when the owning operation finishes.
use std::fs::{File, TryLockError};

pub struct FileLock(File);
impl FileLock {
    pub fn try_exclusive(file: File) -> Result<Self, TryLockError> {
        file.try_lock()?;
        Ok(Self(file))
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        // A concurrently spawned child can inherit the descriptor until exec.
        // Closing our copy alone would leave the resource locked in that window.
        let _ = self.0.unlock();
    }
}

/// Both local and SSH imports share this lock, including sampling-boundary syncs.
pub(crate) fn history(root: &std::path::Path, source_id: &str) -> anyhow::Result<FileLock> {
    let path = root.join(format!("history-{}.lock", crate::models::hash(source_id)));
    let start = std::time::Instant::now();
    loop {
        crate::operations::check()?;
        let file = File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match FileLock::try_exclusive(file) {
            Ok(lock) => return Ok(lock),
            Err(TryLockError::WouldBlock)
                if start.elapsed() < std::time::Duration::from_secs(45) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_with_a_duplicate_handle_after_success_or_error() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("operation.lock");
        for fail in [false, true] {
            let file = File::create(&path).unwrap();
            let inherited = file.try_clone().unwrap();
            let contender = File::options().write(true).open(&path).unwrap();
            let operation = || -> std::io::Result<()> {
                let _lock = FileLock::try_exclusive(file)?;
                assert!(matches!(
                    contender.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                if fail {
                    return Err(std::io::Error::other("fixture failure"));
                }
                Ok(())
            };
            assert_eq!(operation().is_err(), fail);
            let _next = FileLock::try_exclusive(contender).unwrap();
            // Keep the duplicate alive through reacquisition, as a child might.
            drop(inherited);
        }
    }
}
