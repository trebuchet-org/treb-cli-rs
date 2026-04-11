use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use fs2::FileExt;

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Workspace automation tasks")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run Cargo against a specific Foundry backend.
    Foundry(FoundryArgs),
    /// Run Cargo against all Foundry backends sequentially.
    FoundryAll(FoundryAllArgs),
}

const ALL_BACKENDS: [Backend; 3] = [Backend::Nightly, Backend::V1_6_0_Rc1, Backend::V1_5_1];

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum Backend {
    Nightly,
    #[value(name = "v1.6.0-rc1")]
    #[allow(non_camel_case_types)]
    V1_6_0_Rc1,
    #[value(name = "v1.5.1")]
    V1_5_1,
}

impl Backend {
    fn feature_name(self) -> &'static str {
        match self {
            Self::Nightly => "foundry-nightly",
            // rc1 uses the same alloy 1.x non-generic API as v1.5.1
            Self::V1_6_0_Rc1 | Self::V1_5_1 => "foundry-v1-5-1",
        }
    }

    /// Directory under `backends/` containing this backend's manifest.
    /// Returns `None` for nightly (uses root workspace directly).
    fn backend_dir(self) -> Option<&'static str> {
        match self {
            Self::Nightly => None,
            Self::V1_6_0_Rc1 => Some("v1.6.0-rc1"),
            Self::V1_5_1 => Some("v1.5.1"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Nightly => "nightly",
            Self::V1_6_0_Rc1 => "v1.6.0-rc1",
            Self::V1_5_1 => "v1.5.1",
        }
    }

    /// Directory under `patches/` containing source patches for this backend.
    /// Returns `None` if no patches are needed.
    fn patches_dir(self) -> Option<&'static str> {
        match self {
            Self::Nightly => Some("foundry-nightly"),
            Self::V1_6_0_Rc1 | Self::V1_5_1 => None,
        }
    }
}

#[derive(Args)]
struct FoundryArgs {
    /// Foundry backend to build against.
    #[arg(long, value_enum, default_value = "nightly")]
    backend: Backend,
    /// Cargo arguments to run after the backend has been selected.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    cargo_args: Vec<String>,
}

#[derive(Args)]
struct FoundryAllArgs {
    /// Cargo arguments to run against each backend.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    cargo_args: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Foundry(args) => run_foundry_single(args.backend, args.cargo_args),
        Commands::FoundryAll(args) => run_foundry_all(args.cargo_args),
    }
}

fn run_foundry_all(cargo_args: Vec<String>) -> Result<()> {
    let mut failed: Vec<&str> = Vec::new();

    for backend in ALL_BACKENDS {
        eprintln!("\n{}", "=".repeat(60));
        eprintln!("  Backend: {}", backend.label());
        eprintln!("{}\n", "=".repeat(60));

        if let Err(e) = run_foundry_single(backend, cargo_args.clone()) {
            eprintln!("FAILED [{}]: {e:#}", backend.label());
            failed.push(backend.label());
        }
    }

    eprintln!("\n{}", "=".repeat(60));
    if failed.is_empty() {
        eprintln!("  All {} backends passed", ALL_BACKENDS.len());
        eprintln!("{}", "=".repeat(60));
        Ok(())
    } else {
        eprintln!(
            "  {}/{} backends failed: {}",
            failed.len(),
            ALL_BACKENDS.len(),
            failed.join(", ")
        );
        eprintln!("{}", "=".repeat(60));
        bail!("{} backend(s) failed", failed.len());
    }
}

fn run_foundry_single(backend: Backend, cargo_args: Vec<String>) -> Result<()> {
    let root = workspace_root()?;

    // Acquire workspace lock to prevent concurrent manifest swaps
    let _workspace_lock = WorkspaceLock::acquire(&root)?;

    // For alternate backends, swap in the backend's Cargo.toml + Cargo.lock
    let _manifest_guard = match backend.backend_dir() {
        None => None,
        Some(dir) => {
            let backend_dir = root.join("backends").join(dir);
            if !backend_dir.exists() {
                bail!(
                    "backend directory not found: {}\n\
                     Available backends are in the backends/ directory.",
                    backend_dir.display()
                );
            }
            Some(WorkspaceFilesGuard::apply_from_dir(&root, &backend_dir)?)
        }
    };

    // Apply source patches to cargo git checkouts (after manifest swap so the
    // foundry tag in Cargo.toml matches the backend being built).
    if let Some(patches_dir) = backend.patches_dir() {
        apply_cargo_source_patches(&root, patches_dir)?;
    }

    let cargo_args = if cargo_args.is_empty() {
        vec!["check".to_string(), "-p".to_string(), "treb-cli".to_string(), "--tests".to_string()]
    } else {
        cargo_args
    };

    // Insert --no-default-features --features <backend> before any `--` separator
    // so they're treated as cargo flags, not test-binary flags.
    let separator_pos = cargo_args.iter().position(|a| a == "--");
    let mut final_args = Vec::with_capacity(cargo_args.len() + 3);
    let (before, after) = match separator_pos {
        Some(pos) => (&cargo_args[..pos], Some(&cargo_args[pos..])),
        None => (cargo_args.as_slice(), None),
    };
    final_args.extend_from_slice(before);
    final_args.push("--no-default-features".to_string());
    final_args.push("--features".to_string());
    final_args.push(backend.feature_name().to_string());
    if let Some(rest) = after {
        final_args.extend_from_slice(rest);
    }
    let cargo_args = final_args;

    eprintln!("xtask: cargo {} [{}]", cargo_args.join(" "), backend.label());

    let status = Command::new("cargo")
        .args(&cargo_args)
        .current_dir(&root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to start cargo")?;

    if !status.success() {
        bail!("cargo command failed with status {status}");
    }

    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| anyhow!("xtask must live under the workspace root"))?;
    Ok(root.to_path_buf())
}

/// RAII guard that swaps the workspace Cargo.toml and Cargo.lock with a
/// backend's versions, then restores the originals on drop.
struct WorkspaceFilesGuard {
    cargo_toml_path: PathBuf,
    cargo_lock_path: PathBuf,
    original_cargo_toml: Vec<u8>,
    original_cargo_lock: Vec<u8>,
}

impl WorkspaceFilesGuard {
    /// Read Cargo.toml + Cargo.lock from `backend_dir` and swap them into the
    /// workspace root.
    fn apply_from_dir(root: &Path, backend_dir: &Path) -> Result<Self> {
        let cargo_toml_path = root.join("Cargo.toml");
        let cargo_lock_path = root.join("Cargo.lock");

        let original_cargo_toml =
            fs::read(&cargo_toml_path).context("failed to read workspace Cargo.toml")?;
        let original_cargo_lock =
            fs::read(&cargo_lock_path).context("failed to read workspace Cargo.lock")?;

        let backend_cargo_toml = fs::read(backend_dir.join("Cargo.toml"))
            .with_context(|| format!("failed to read {}/Cargo.toml", backend_dir.display()))?;
        let backend_cargo_lock = fs::read(backend_dir.join("Cargo.lock"))
            .with_context(|| format!("failed to read {}/Cargo.lock", backend_dir.display()))?;

        fs::write(&cargo_toml_path, backend_cargo_toml)
            .context("failed to write backend Cargo.toml")?;
        fs::write(&cargo_lock_path, backend_cargo_lock)
            .context("failed to write backend Cargo.lock")?;

        Ok(Self { cargo_toml_path, cargo_lock_path, original_cargo_toml, original_cargo_lock })
    }
}

impl Drop for WorkspaceFilesGuard {
    fn drop(&mut self) {
        let _ = fs::write(&self.cargo_toml_path, &self.original_cargo_toml);
        let _ = fs::write(&self.cargo_lock_path, &self.original_cargo_lock);
    }
}

/// Workspace-level lock to prevent concurrent xtask runs from stepping on
/// each other's manifest swaps.
struct WorkspaceLock {
    _file: fs::File,
}

impl WorkspaceLock {
    fn acquire(root: &Path) -> Result<Self> {
        let xtask_dir = root.join("target").join("xtask");
        fs::create_dir_all(&xtask_dir).context("failed to create target/xtask for lock")?;
        let lock_path = xtask_dir.join("foundry-compat.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("failed to open xtask lock file {}", lock_path.display()))?;
        file.lock_exclusive().with_context(|| format!("failed to lock {}", lock_path.display()))?;
        Ok(Self { _file: file })
    }
}

// ---------------------------------------------------------------------------
// Cargo source patching
// ---------------------------------------------------------------------------

/// Apply `.patch` files from `patches/<dir>/` to the cargo git checkouts.
///
/// Patches are applied idempotently: if a patch is already applied (detected by
/// checking whether the reverse patch applies cleanly), it is skipped.
///
/// The patches target the cargo git checkout of the `foundry-rs/foundry` repo
/// located under `$CARGO_HOME/git/checkouts/foundry-*/`. The correct checkout
/// subdirectory is identified by matching the foundry git tag from the active
/// workspace `Cargo.toml`.
fn apply_cargo_source_patches(root: &Path, patches_dir: &str) -> Result<()> {
    let patches_path = root.join("patches").join(patches_dir);
    if !patches_path.exists() {
        return Ok(());
    }

    let mut patches: Vec<_> = fs::read_dir(&patches_path)
        .with_context(|| format!("failed to read patches dir {}", patches_path.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "patch"))
        .map(|e| e.path())
        .collect();
    patches.sort();

    if patches.is_empty() {
        return Ok(());
    }

    let checkout_dir = find_foundry_checkout(root)?;

    for patch_path in &patches {
        let patch_name = patch_path.file_name().unwrap().to_string_lossy();

        // Check if already applied (reverse patch applies cleanly)
        let reverse_check = Command::new("git")
            .args(["apply", "-R", "--check"])
            .arg(patch_path)
            .current_dir(&checkout_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if reverse_check.map_or(false, |s| s.success()) {
            eprintln!("xtask: patch already applied: {patch_name}");
            // Touch patched files so cargo knows to recompile
            invalidate_forge_script_cache(root)?;
            continue;
        }

        // Check if patch applies forward
        let forward_check = Command::new("git")
            .args(["apply", "--check"])
            .arg(patch_path)
            .current_dir(&checkout_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if !forward_check.map_or(false, |s| s.success()) {
            bail!("patch does not apply cleanly: {patch_name}");
        }

        // Apply the patch
        let status = Command::new("git")
            .args(["apply"])
            .arg(patch_path)
            .current_dir(&checkout_dir)
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| format!("failed to run git apply for {patch_name}"))?;

        if !status.success() {
            bail!("git apply failed for {patch_name}");
        }

        eprintln!("xtask: applied patch: {patch_name}");
        invalidate_forge_script_cache(root)?;
    }

    Ok(())
}

/// Invalidate cargo's fingerprint for the forge-script crate after patching.
/// Cargo fingerprints git dependency builds by source hash; we need to remove
/// the cached compilation artifacts so the patched source gets recompiled.
fn invalidate_forge_script_cache(root: &Path) -> Result<()> {
    // Remove incremental compilation artifacts for forge-script from
    // all target directories that might be in use.
    for target_dir in &["target", "target/v1.5.1", "target/v1.6.0-rc1"] {
        let deps_dir = root.join(target_dir).join("debug/.fingerprint");
        if !deps_dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&deps_dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("forge-script-") {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(())
}

/// Locate the cargo git checkout for `foundry-rs/foundry` matching the tag in
/// the (possibly swapped) workspace `Cargo.toml`.
///
/// Cargo stores git checkouts at `$CARGO_HOME/git/checkouts/foundry-<db-hash>/<short-rev>/`.
/// We resolve the tag to a commit hash via `Cargo.lock` (which records the
/// exact `#<hash>` in the source URL) and match against subdirectory names.
fn find_foundry_checkout(root: &Path) -> Result<PathBuf> {
    let cargo_home = std::env::var("CARGO_HOME")
        .unwrap_or_else(|_| format!("{}/.cargo", std::env::var("HOME").unwrap_or_default()));
    let checkouts_dir = Path::new(&cargo_home).join("git/checkouts");

    let foundry_parent = fs::read_dir(&checkouts_dir)
        .context("failed to read cargo git checkouts")?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("foundry-"))
        .ok_or_else(|| anyhow!("no foundry checkout found in {}", checkouts_dir.display()))?
        .path();

    // Extract the resolved foundry commit hash from Cargo.lock.
    // Lock entries look like: source = "git+https://...foundry?tag=...#<full-hash>"
    let cargo_lock = fs::read_to_string(root.join("Cargo.lock"))
        .context("failed to read Cargo.lock")?;

    let resolved_hash = cargo_lock
        .lines()
        .find(|l| l.contains("foundry-rs/foundry") && l.contains('#'))
        .and_then(|l| l.rsplit('#').next())
        .map(|h| h.trim_end_matches('"').to_string())
        .ok_or_else(|| anyhow!("could not extract foundry commit hash from Cargo.lock"))?;

    // Cargo checkout dirs are named by short (7-char) commit hash
    let short = &resolved_hash[..7.min(resolved_hash.len())];

    let checkout_dir = foundry_parent.join(short);
    if checkout_dir.is_dir() {
        return Ok(checkout_dir);
    }

    // Fallback: scan subdirectories for a matching HEAD
    for entry in fs::read_dir(&foundry_parent)
        .context("failed to read foundry checkout directory")?
        .filter_map(|e| e.ok())
    {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if resolved_hash.starts_with(&entry.file_name().to_string_lossy().to_string()) {
            return Ok(dir);
        }
    }

    bail!(
        "no matching checkout found for commit {} in {}",
        short,
        foundry_parent.display()
    )
}
