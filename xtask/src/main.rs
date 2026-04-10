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
