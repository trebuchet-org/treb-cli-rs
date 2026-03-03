//! Atomic file I/O primitives and advisory file locking for the registry.

use std::fs;
use std::io::Write;
use std::path::Path;

use fs2::FileExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tempfile::NamedTempFile;
use treb_core::error::TrebError;

/// Read and deserialize a JSON file into `T`.
///
/// Returns `TrebError::Io` for I/O failures and `TrebError::Registry` for
/// deserialization failures.
pub fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, TrebError> {
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(|e| {
        TrebError::Registry(format!("failed to parse {}: {e}", path.display()))
    })
}

/// Read and deserialize a JSON file, returning `T::default()` if the file does
/// not exist.
pub fn read_json_file_or_default<T: DeserializeOwned + Default>(
    path: &Path,
) -> Result<T, TrebError> {
    if !path.exists() {
        return Ok(T::default());
    }
    read_json_file(path)
}

/// Atomically write `value` as 2-space-indented JSON with a trailing newline.
///
/// Creates parent directories if they don't exist. Writes to a temporary file
/// in the same directory and then persists (renames) it to `path`, ensuring no
/// partial writes are visible.
pub fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), TrebError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut json = serde_json::to_string_pretty(value).map_err(|e| {
        TrebError::Registry(format!("failed to serialize to {}: {e}", path.display()))
    })?;
    json.push('\n');

    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(json.as_bytes())?;
    tmp.persist(path).map_err(|e| {
        TrebError::Registry(format!("failed to persist temp file to {}: {e}", path.display()))
    })?;

    Ok(())
}

/// RAII guard that releases an exclusive advisory lock on drop.
///
/// On POSIX, the advisory lock is released when the file descriptor is closed
/// (i.e. when `File` is dropped). We use `fs2::FileExt::unlock` via
/// fully-qualified syntax to avoid the std `File::unlock` (stabilised in 1.89,
/// above our MSRV).
struct FileLockGuard {
    file: fs::File,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Acquire an exclusive advisory lock on `<path>.lock`, execute `f`, and
/// release the lock when the guard drops.
pub fn with_file_lock<F, T>(path: &Path, f: F) -> Result<T, TrebError>
where
    F: FnOnce() -> Result<T, TrebError>,
{
    let lock_path = path.with_extension("lock");

    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;

    file.lock_exclusive()?;
    let _guard = FileLockGuard { file };

    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Instant;

    use serde::Deserialize;
    use tempfile::TempDir;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    #[test]
    fn write_creates_correct_json_with_trailing_newline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sample.json");

        let sample = Sample {
            name: "hello".into(),
            count: 42,
        };
        write_json_file(&path, &sample).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.ends_with('\n'), "JSON should end with trailing newline");

        // Verify 2-space indentation (serde_json::to_string_pretty default)
        assert!(raw.contains("  \"name\""));

        let parsed: Sample = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, sample);
    }

    #[test]
    fn write_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a").join("b").join("c").join("data.json");

        let sample = Sample {
            name: "nested".into(),
            count: 1,
        };
        write_json_file(&path, &sample).unwrap();

        assert!(path.exists());
        let parsed: Sample = read_json_file(&path).unwrap();
        assert_eq!(parsed, sample);
    }

    #[test]
    fn read_nonexistent_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.json");

        let result: Sample = read_json_file_or_default(&path).unwrap();
        assert_eq!(result, Sample::default());
    }

    #[test]
    fn invalid_json_returns_descriptive_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, "not valid json {{{").unwrap();

        let result = read_json_file::<Sample>(&path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("failed to parse") && msg.contains("bad.json"),
            "error should mention file path: {msg}"
        );
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("round.json");

        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        map.insert("primes".into(), vec![2, 3, 5, 7, 11]);
        map.insert("fibs".into(), vec![1, 1, 2, 3, 5]);

        write_json_file(&path, &map).unwrap();
        let loaded: HashMap<String, Vec<u32>> = read_json_file(&path).unwrap();
        assert_eq!(loaded, map);
    }

    #[test]
    fn concurrent_lock_blocks() {
        let dir = TempDir::new().unwrap();
        let lock_target = dir.path().join("data.json");

        let barrier = Arc::new(Barrier::new(2));
        let lock_target_clone = lock_target.clone();
        let barrier_clone = Arc::clone(&barrier);

        // Thread 1: acquire lock, signal ready, hold for 200ms
        let t1 = thread::spawn(move || {
            with_file_lock(&lock_target_clone, || {
                barrier_clone.wait();
                thread::sleep(std::time::Duration::from_millis(200));
                Ok(())
            })
            .unwrap();
        });

        // Thread 2: wait for thread 1 to hold the lock, then try to acquire
        barrier.wait();
        // Small delay to ensure t1's lock is held
        thread::sleep(std::time::Duration::from_millis(20));

        let start = Instant::now();
        with_file_lock(&lock_target, || Ok(())).unwrap();
        let waited = start.elapsed();

        t1.join().unwrap();

        assert!(
            waited.as_millis() >= 50,
            "thread 2 should have blocked waiting for lock, only waited {waited:?}"
        );
    }
}
