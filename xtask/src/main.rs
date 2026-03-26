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
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum Backend {
    Nightly,
    #[value(name = "v1.5.1")]
    V1_5_1,
}

impl Backend {
    fn feature_name(self) -> &'static str {
        match self {
            Self::Nightly => "foundry-nightly",
            Self::V1_5_1 => "foundry-v1-5-1",
        }
    }
}

#[derive(Args)]
struct FoundryArgs {
    /// Foundry backend to build against.
    #[arg(long, value_enum, default_value = "nightly")]
    backend: Backend,
    /// Git ref containing the vetted root manifest/lock for the alternate backend.
    #[arg(long, default_value = "release/foundry-v1.5.1")]
    release_ref: String,
    /// Cargo arguments to run after the backend has been selected.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    cargo_args: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Foundry(args) => run_foundry(args),
    }
}

fn run_foundry(args: FoundryArgs) -> Result<()> {
    let root = workspace_root()?;
    let _workspace_lock = WorkspaceLock::acquire(&root)?;

    let _manifest_guard = match args.backend {
        Backend::Nightly => None,
        Backend::V1_5_1 => Some(WorkspaceFilesGuard::apply_git_ref(&root, &args.release_ref)?),
    };

    let mut cargo_args = if args.cargo_args.is_empty() {
        vec!["check".to_string(), "-p".to_string(), "treb-cli".to_string(), "--tests".to_string()]
    } else {
        args.cargo_args
    };

    cargo_args.push("--no-default-features".to_string());
    cargo_args.push("--features".to_string());
    cargo_args.push(args.backend.feature_name().to_string());

    eprintln!(
        "xtask: cargo {} [{}]",
        cargo_args.join(" "),
        match args.backend {
            Backend::Nightly => "nightly",
            Backend::V1_5_1 => "v1.5.1",
        }
    );

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

struct WorkspaceFilesGuard {
    cargo_toml_path: PathBuf,
    cargo_lock_path: PathBuf,
    original_cargo_toml: Vec<u8>,
    original_cargo_lock: Vec<u8>,
}

impl WorkspaceFilesGuard {
    fn apply_git_ref(root: &Path, git_ref: &str) -> Result<Self> {
        let cargo_toml_path = root.join("Cargo.toml");
        let cargo_lock_path = root.join("Cargo.lock");

        let original_cargo_toml =
            fs::read(&cargo_toml_path).context("failed to read workspace Cargo.toml")?;
        let original_cargo_lock =
            fs::read(&cargo_lock_path).context("failed to read workspace Cargo.lock")?;

        let backend_cargo_toml = git_show(root, git_ref, "Cargo.toml")
            .with_context(|| format!("failed to load Cargo.toml from git ref {git_ref}"))?;
        let backend_cargo_lock = git_show(root, git_ref, "Cargo.lock")
            .with_context(|| format!("failed to load Cargo.lock from git ref {git_ref}"))?;

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

fn git_show(root: &Path, git_ref: &str, path: &str) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(["show", &format!("{git_ref}:{path}")])
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("failed to run git show for {git_ref}:{path}"))?;

    if !output.status.success() {
        bail!("git show failed for {git_ref}:{path}");
    }

    Ok(output.stdout)
}

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
