//! `treb tag` command implementation.

use std::env;

use anyhow::{Context, bail};
use owo_colors::{OwoColorize, Style};
use serde::Serialize;
use treb_registry::Registry;

use crate::{
    commands::resolve::resolve_deployment,
    output,
    ui::{color, selector::fuzzy_select_deployment_id},
};

/// JSON output for tag operations.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TagOutputJson {
    deployment_id: String,
    action: String,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

/// Apply a color style when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

pub async fn run(
    deployment_query: Option<String>,
    add: Option<String>,
    remove: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(".treb").exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }

    let mut registry = Registry::open(&cwd).context("failed to open registry")?;
    let lookup = registry.load_lookup_index().context("failed to load lookup index")?;

    let query = match deployment_query {
        Some(q) => q,
        None => {
            let deployments: Vec<_> = registry.list_deployments().into_iter().cloned().collect();
            fuzzy_select_deployment_id(&deployments)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("no deployment selected"))?
        }
    };

    let resolved = resolve_deployment(&query, &registry, &lookup)?;
    let deployment_id = resolved.id.clone();

    match (add, remove) {
        (Some(tag), None) => add_tag(&mut registry, &deployment_id, &tag, json),
        (None, Some(tag)) => remove_tag(&mut registry, &deployment_id, &tag, json),
        (None, None) => show_tags(&registry, &deployment_id, json),
        // clap's conflicts_with prevents this, but handle it defensively
        (Some(_), Some(_)) => bail!("--add and --remove cannot be used together"),
    }
}

fn show_tags(registry: &Registry, deployment_id: &str, json: bool) -> anyhow::Result<()> {
    let dep = registry.get_deployment(deployment_id).unwrap();
    let mut tags: Vec<String> = dep.tags.clone().unwrap_or_default();

    if json {
        output::print_json(&TagOutputJson {
            deployment_id: deployment_id.to_string(),
            action: "show".to_string(),
            tags,
            tag: None,
        })?;
    } else {
        println!();
        println!("{} {}", styled("Deployment:", color::STAGE), styled(deployment_id, color::STAGE),);
        println!(
            "{} {}",
            styled("Address:", color::SECTION_HEADER),
            styled(&dep.address, color::SUCCESS),
        );
        tags.sort();
        let tags_value = if tags.is_empty() {
            styled("No tags", color::GRAY)
        } else {
            tags.iter().map(|t| styled(t, color::CYAN)).collect::<Vec<_>>().join(", ")
        };
        println!("{} {tags_value}", styled("Tags:   ", color::SECTION_HEADER));
        println!();
    }

    Ok(())
}

fn add_tag(
    registry: &mut Registry,
    deployment_id: &str,
    tag: &str,
    json: bool,
) -> anyhow::Result<()> {
    let dep = registry.get_deployment(deployment_id).unwrap();
    let existing_tags = dep.tags.clone().unwrap_or_default();

    if existing_tags.contains(&tag.to_string()) {
        bail!("tag '{}' already exists on deployment '{}'", tag, deployment_id);
    }

    let mut dep = dep.clone();
    let mut tags = existing_tags;
    tags.push(tag.to_string());
    dep.tags = Some(tags.clone());
    registry.update_deployment(dep)?;

    if json {
        output::print_json(&TagOutputJson {
            deployment_id: deployment_id.to_string(),
            action: "add".to_string(),
            tags,
            tag: Some(tag.to_string()),
        })?;
    } else {
        println!(
            "{}",
            styled(&format!("\u{2705} Added tag '{tag}' to {deployment_id}"), color::GREEN,)
        );
        tags.sort();
        let tags_display =
            tags.iter().map(|t| styled(t, color::CYAN)).collect::<Vec<_>>().join(", ");
        println!();
        println!("Current tags: {tags_display}");
    }

    Ok(())
}

fn remove_tag(
    registry: &mut Registry,
    deployment_id: &str,
    tag: &str,
    json: bool,
) -> anyhow::Result<()> {
    let dep = registry.get_deployment(deployment_id).unwrap();
    let existing_tags = dep.tags.clone().unwrap_or_default();

    if !existing_tags.contains(&tag.to_string()) {
        bail!("tag '{}' not found on deployment '{}'", tag, deployment_id);
    }

    let mut dep = dep.clone();
    let mut tags = existing_tags;
    tags.retain(|t| t != tag);
    dep.tags = if tags.is_empty() { None } else { Some(tags.clone()) };
    registry.update_deployment(dep)?;

    let mut final_tags = tags;

    if json {
        output::print_json(&TagOutputJson {
            deployment_id: deployment_id.to_string(),
            action: "remove".to_string(),
            tags: final_tags,
            tag: Some(tag.to_string()),
        })?;
    } else {
        println!(
            "{}",
            styled(&format!("\u{2705} Removed tag {tag} from {deployment_id}"), color::GREEN,)
        );
        final_tags.sort();
        let tags_value = if final_tags.is_empty() {
            styled("No tags", color::GRAY)
        } else {
            final_tags.iter().map(|t| styled(t, color::CYAN)).collect::<Vec<_>>().join(", ")
        };
        println!();
        println!("Remaining tags: {tags_value}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType,
        VerificationInfo, VerificationStatus,
    };
    use treb_registry::Registry;

    fn make_deployment(id: &str, tags: Option<Vec<String>>) -> Deployment {
        Deployment {
            id: id.into(),
            namespace: "mainnet".into(),
            chain_id: 42220,
            contract_name: "Counter".into(),
            label: "v1.0.0".into(),
            address: "0x42eDa75c4AC3fCf6eA20D091Ad1Ff79e9c52833D".into(),
            deployment_type: DeploymentType::Singleton,
            transaction_id: "tx-001".into(),
            deployment_strategy: DeploymentStrategy {
                method: DeploymentMethod::Create,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: String::new(),
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: None,
            artifact: ArtifactInfo {
                path: "out/Counter.json".into(),
                compiler_version: "0.8.24".into(),
                bytecode_hash: "0xabc".into(),
                script_path: "script/Deploy.s.sol".into(),
                git_commit: "abc1234".into(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags,
            created_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
        }
    }

    fn setup_registry(tags: Option<Vec<String>>) -> (TempDir, Registry) {
        let tmp = TempDir::new().unwrap();
        let mut registry = Registry::init(tmp.path()).unwrap();
        let dep = make_deployment("mainnet/42220/Counter:v1.0.0", tags);
        registry.insert_deployment(dep).unwrap();
        (tmp, registry)
    }

    #[test]
    fn show_tags_empty() {
        let (_tmp, registry) = setup_registry(None);
        let result = super::show_tags(&registry, "mainnet/42220/Counter:v1.0.0", false);
        assert!(result.is_ok());
    }

    #[test]
    fn show_tags_with_existing() {
        let (_tmp, registry) = setup_registry(Some(vec!["v1.0.0".into(), "stable".into()]));
        let result = super::show_tags(&registry, "mainnet/42220/Counter:v1.0.0", false);
        assert!(result.is_ok());
    }

    #[test]
    fn show_tags_json() {
        let (_tmp, registry) = setup_registry(Some(vec!["v1.0.0".into()]));
        let result = super::show_tags(&registry, "mainnet/42220/Counter:v1.0.0", true);
        assert!(result.is_ok());
    }

    #[test]
    fn add_tag_success() {
        let (_tmp, mut registry) = setup_registry(None);
        let result = super::add_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v2.0.0", false);
        assert!(result.is_ok());

        let dep = registry.get_deployment("mainnet/42220/Counter:v1.0.0").unwrap();
        assert_eq!(dep.tags, Some(vec!["v2.0.0".to_string()]));
    }

    #[test]
    fn add_tag_persisted_to_disk() {
        let (tmp, mut registry) = setup_registry(None);
        super::add_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v2.0.0", false).unwrap();

        // Re-open registry from disk
        let registry2 = Registry::open(tmp.path()).unwrap();
        let dep = registry2.get_deployment("mainnet/42220/Counter:v1.0.0").unwrap();
        assert_eq!(dep.tags, Some(vec!["v2.0.0".to_string()]));
    }

    #[test]
    fn add_tag_duplicate_error() {
        let (_tmp, mut registry) = setup_registry(Some(vec!["v2.0.0".into()]));
        let result = super::add_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v2.0.0", false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already exists"));
        assert!(err.contains("v2.0.0"));
        assert!(err.contains("Counter"));
    }

    #[test]
    fn add_tag_json() {
        let (_tmp, mut registry) = setup_registry(None);
        let result = super::add_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v2.0.0", true);
        assert!(result.is_ok());
    }

    #[test]
    fn remove_tag_success() {
        let (_tmp, mut registry) = setup_registry(Some(vec!["v1.0.0".into(), "stable".into()]));
        let result =
            super::remove_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v1.0.0", false);
        assert!(result.is_ok());

        let dep = registry.get_deployment("mainnet/42220/Counter:v1.0.0").unwrap();
        assert_eq!(dep.tags, Some(vec!["stable".to_string()]));
    }

    #[test]
    fn remove_last_tag_sets_none() {
        let (_tmp, mut registry) = setup_registry(Some(vec!["v1.0.0".into()]));
        let result =
            super::remove_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v1.0.0", false);
        assert!(result.is_ok());

        let dep = registry.get_deployment("mainnet/42220/Counter:v1.0.0").unwrap();
        assert_eq!(dep.tags, None);
    }

    #[test]
    fn remove_tag_persisted_to_disk() {
        let (tmp, mut registry) = setup_registry(Some(vec!["v1.0.0".into(), "stable".into()]));
        super::remove_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v1.0.0", false).unwrap();

        // Re-open registry from disk
        let registry2 = Registry::open(tmp.path()).unwrap();
        let dep = registry2.get_deployment("mainnet/42220/Counter:v1.0.0").unwrap();
        assert_eq!(dep.tags, Some(vec!["stable".to_string()]));
    }

    #[test]
    fn remove_tag_not_found_error() {
        let (_tmp, mut registry) = setup_registry(None);
        let result =
            super::remove_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "nonexistent", false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
        assert!(err.contains("nonexistent"));
        assert!(err.contains("Counter"));
    }

    #[test]
    fn remove_tag_json() {
        let (_tmp, mut registry) = setup_registry(Some(vec!["v1.0.0".into()]));
        let result =
            super::remove_tag(&mut registry, "mainnet/42220/Counter:v1.0.0", "v1.0.0", true);
        assert!(result.is_ok());
    }
}
