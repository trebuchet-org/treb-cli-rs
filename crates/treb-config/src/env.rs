//! `.env` file loading via dotenvy.
//!
//! Loads `.env` then `.env.local` from the project root, with `.env.local`
//! values overriding `.env`. Missing files are silently ignored.

use std::path::Path;

/// Load `.env` and `.env.local` files from `project_root`.
///
/// `.env` is loaded first, then `.env.local` overrides any overlapping
/// variables. Missing files are silently skipped — this is not an error.
///
/// Must be called before any config parsing that uses env var expansion.
pub fn load_dotenv(project_root: &Path) {
    // Load .env first (base values).
    let env_path = project_root.join(".env");
    let _ = dotenvy::from_path_override(&env_path);

    // Load .env.local second (overrides .env values).
    let local_path = project_root.join(".env.local");
    let _ = dotenvy::from_path_override(&local_path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::TempDir;

    #[test]
    fn load_dotenv_sets_env_vars() {
        let tmp = TempDir::new().unwrap();
        let key = "TREB_TEST_DOTENV_BASIC_12345";
        std::fs::write(tmp.path().join(".env"), format!("{key}=hello_from_env\n")).unwrap();

        load_dotenv(tmp.path());

        assert_eq!(env::var(key).unwrap(), "hello_from_env");
        // SAFETY: test uses a unique env var name, no other threads access it.
        unsafe { env::remove_var(key) };
    }

    #[test]
    fn load_dotenv_local_overrides_env() {
        let tmp = TempDir::new().unwrap();
        let key = "TREB_TEST_DOTENV_OVERRIDE_67890";
        std::fs::write(tmp.path().join(".env"), format!("{key}=from_env\n")).unwrap();
        std::fs::write(tmp.path().join(".env.local"), format!("{key}=from_local\n")).unwrap();

        load_dotenv(tmp.path());

        assert_eq!(env::var(key).unwrap(), "from_local");
        // SAFETY: test uses a unique env var name, no other threads access it.
        unsafe { env::remove_var(key) };
    }

    #[test]
    fn load_dotenv_missing_files_does_not_error() {
        let tmp = TempDir::new().unwrap();
        // No .env or .env.local created — should not panic.
        load_dotenv(tmp.path());
    }
}
