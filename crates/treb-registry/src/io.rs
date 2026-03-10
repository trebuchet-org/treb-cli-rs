//! Atomic file I/O primitives and advisory file locking for the registry.

use std::{fs, io::Write, path::Path};

use fs2::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tempfile::NamedTempFile;
use treb_core::error::TrebError;

/// Versioned wrapper used for persisted registry store files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VersionedStore<T> {
    #[serde(rename = "_format")]
    pub format: String,
    pub entries: T,
}

impl<T> VersionedStore<T> {
    /// Wrap `entries` with the current store format marker.
    pub fn new(entries: T) -> Self {
        Self { format: crate::STORE_FORMAT.to_string(), entries }
    }
}

/// Read and deserialize a JSON file into `T`.
///
/// Returns `TrebError::Io` for I/O failures and `TrebError::Registry` for
/// deserialization failures.
pub fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, TrebError> {
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents)
        .map_err(|e| TrebError::Registry(format!("failed to parse {}: {e}", path.display())))
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

/// Read a versioned store file, accepting both wrapped and legacy bare JSON.
///
/// Returns `T::default()` when the file does not exist or a wrapped payload is
/// incompatible/corrupt.
pub fn read_versioned_file<T: DeserializeOwned + Default>(path: &Path) -> Result<T, TrebError> {
    if !path.exists() {
        return Ok(T::default());
    }

    let value: serde_json::Value = read_json_file(path)?;
    let looks_wrapped = value
        .as_object()
        .is_some_and(|object| object.contains_key("_format") && object.contains_key("entries"));

    if looks_wrapped {
        return match serde_json::from_value::<VersionedStore<T>>(value) {
            Ok(wrapped) => Ok(wrapped.entries),
            Err(_) => Ok(T::default()),
        };
    }

    deserialize_value(path, value)
}

/// Read a versioned store file, falling back to the legacy filename when the
/// current filename does not exist yet.
pub fn read_versioned_file_compat<T: DeserializeOwned + Default>(
    path: &Path,
) -> Result<T, TrebError> {
    if path.exists() {
        return read_versioned_file(path);
    }

    if let Some(legacy_path) = crate::legacy_registry_store_path(path) {
        if legacy_path.exists() {
            return read_versioned_file(&legacy_path);
        }
    }

    Ok(T::default())
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

/// Atomically write `entries` under the versioned store wrapper while holding
/// the store's advisory file lock.
pub fn write_versioned_file<T: Serialize>(path: &Path, entries: &T) -> Result<(), TrebError> {
    with_file_lock(path, || {
        let wrapped = VersionedStore::new(entries);
        write_json_file(path, &wrapped)
    })
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

    let file = fs::OpenOptions::new().create(true).write(true).truncate(false).open(&lock_path)?;

    file.lock_exclusive()?;
    let _guard = FileLockGuard { file };

    f()
}

fn deserialize_value<T: DeserializeOwned>(
    path: &Path,
    value: serde_json::Value,
) -> Result<T, TrebError> {
    serde_json::from_value(value)
        .map_err(|e| TrebError::Registry(format!("failed to parse {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        collections::HashMap,
        sync::{Arc, Barrier},
        thread,
        time::Instant,
    };

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

        let sample = Sample { name: "hello".into(), count: 42 };
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

        let sample = Sample { name: "nested".into(), count: 1 };
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
    fn versioned_store_serializes_with_format_and_entries() {
        let wrapped = VersionedStore::new(Sample { name: "hello".into(), count: 42 });

        let json = serde_json::to_value(&wrapped).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "_format": crate::STORE_FORMAT,
                "entries": {
                    "name": "hello",
                    "count": 42
                }
            })
        );
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
    fn read_versioned_file_reads_wrapped_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wrapped.json");

        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        map.insert("primes".into(), vec![2, 3, 5, 7, 11]);

        write_json_file(&path, &VersionedStore::new(map.clone())).unwrap();

        let loaded: HashMap<String, Vec<u32>> = read_versioned_file(&path).unwrap();
        assert_eq!(loaded, map);
    }

    #[test]
    fn read_versioned_file_ignores_unknown_wrapped_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wrapped-unknown-format.json");

        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        map.insert("primes".into(), vec![2, 3, 5, 7, 11]);

        write_json_file(
            &path,
            &serde_json::json!({
                "_format": "treb-v999",
                "entries": map,
            }),
        )
        .unwrap();

        let loaded: HashMap<String, Vec<u32>> = read_versioned_file(&path).unwrap();
        assert_eq!(loaded["primes"], vec![2, 3, 5, 7, 11]);
    }

    #[test]
    fn read_versioned_file_returns_default_for_incompatible_wrapped_payload() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wrapped-incompatible.json");

        write_json_file(
            &path,
            &serde_json::json!({
                "_format": "treb-v999",
                "entries": [1, 2, 3],
            }),
        )
        .unwrap();

        let loaded: HashMap<String, Vec<u32>> = read_versioned_file(&path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn read_versioned_file_reads_bare_map_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bare.json");

        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        map.insert("fibs".into(), vec![1, 1, 2, 3, 5]);

        write_json_file(&path, &map).unwrap();

        let loaded: HashMap<String, Vec<u32>> = read_versioned_file(&path).unwrap();
        assert_eq!(loaded, map);
    }

    #[test]
    fn read_versioned_file_reads_bare_map_with_format_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bare-with-format-key.json");

        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        map.insert("_format".into(), vec![1, 2, 3]);
        map.insert("fibs".into(), vec![1, 1, 2, 3, 5]);

        write_json_file(&path, &map).unwrap();

        let loaded: HashMap<String, Vec<u32>> = read_versioned_file(&path).unwrap();
        assert_eq!(loaded, map);
    }

    #[test]
    fn read_versioned_file_missing_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing-versioned.json");

        let loaded: HashMap<String, Vec<u32>> = read_versioned_file(&path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn write_versioned_file_writes_wrapped_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("versioned.json");

        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        map.insert("evens".into(), vec![2, 4, 6]);

        write_versioned_file(&path, &map).unwrap();

        let json: serde_json::Value = read_json_file(&path).unwrap();
        assert_eq!(json["_format"], crate::STORE_FORMAT);
        assert_eq!(json["entries"]["evens"], serde_json::json!([2, 4, 6]));
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

    #[test]
    fn write_versioned_file_waits_for_existing_lock() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("versioned-lock.json");

        let barrier = Arc::new(Barrier::new(2));
        let path_clone = path.clone();
        let barrier_clone = Arc::clone(&barrier);

        let t1 = thread::spawn(move || {
            with_file_lock(&path_clone, || {
                barrier_clone.wait();
                thread::sleep(std::time::Duration::from_millis(200));
                Ok(())
            })
            .unwrap();
        });

        barrier.wait();
        thread::sleep(std::time::Duration::from_millis(20));

        let start = Instant::now();
        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        map.insert("held".into(), vec![1]);
        write_versioned_file(&path, &map).unwrap();
        let waited = start.elapsed();

        t1.join().unwrap();

        assert!(
            waited.as_millis() >= 50,
            "write_versioned_file should wait for the same advisory lock, only waited {waited:?}"
        );
    }
}
