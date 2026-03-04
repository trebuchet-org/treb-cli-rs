//! Fuzzy-search selectors for deployments and networks.

use console::Term;
use treb_core::{Result, TrebError, types::Deployment};

/// Present a fuzzy-search selector for a list of deployments.
///
/// Returns `Ok(None)` when the list is empty or the user aborts (Esc/q).
/// Returns `Err(TrebError::Cli(...))` when not running in a TTY and no
/// query is supplied.
pub fn fuzzy_select_deployment<'a>(
    deployments: &'a [Deployment],
    query: Option<&str>,
) -> Result<Option<&'a Deployment>> {
    if deployments.is_empty() {
        return Ok(None);
    }

    let is_tty = Term::stdout().is_term();
    if !is_tty && query.is_none() {
        return Err(TrebError::Cli(
            "no TTY detected and no deployment query supplied; \
             pass a deployment ID or run interactively"
                .into(),
        ));
    }

    // Build display strings: "<id>  <contract_name>  <address>"
    let items: Vec<String> = deployments
        .iter()
        .map(|d| format!("{}  {}  {}", d.id, d.contract_name, d.address))
        .collect();

    // If a query is provided, do fuzzy filtering instead of an interactive prompt.
    if let Some(q) = query {
        let matched = fuzzy_filter(&items, q);
        if let Some(first_idx) = matched.first() {
            return Ok(deployments.get(*first_idx));
        }
        return Ok(None);
    }

    // Interactive selection via dialoguer::FuzzySelect.
    let selection = dialoguer::FuzzySelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Select deployment")
        .items(&items)
        .default(0)
        .interact_on_opt(&Term::stdout())
        .map_err(|e| TrebError::Cli(format!("selector error: {e}")))?;

    Ok(selection.and_then(|i| deployments.get(i)))
}

/// Present a fuzzy-search selector for a list of network names.
///
/// Returns `Ok(None)` when the list is empty or the user aborts.
/// Returns `Err(TrebError::Cli(...))` when not running in a TTY.
pub fn fuzzy_select_network(networks: &[String]) -> Result<Option<&str>> {
    if networks.is_empty() {
        return Ok(None);
    }

    let is_tty = Term::stdout().is_term();
    if !is_tty {
        return Err(TrebError::Cli(
            "no TTY detected; pass --network explicitly or run interactively".into(),
        ));
    }

    let selection = dialoguer::FuzzySelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Select network")
        .items(networks)
        .default(0)
        .interact_on_opt(&Term::stdout())
        .map_err(|e| TrebError::Cli(format!("selector error: {e}")))?;

    Ok(selection.and_then(|i| networks.get(i).map(|s| s.as_str())))
}

/// Present a multi-select for a list of deployments.
///
/// Returns `Err(TrebError::Cli(...))` when not running in a TTY.
pub fn multiselect_deployments<'a>(
    deployments: &'a [Deployment],
    prompt: &str,
) -> Result<Vec<&'a Deployment>> {
    let is_tty = Term::stdout().is_term();
    if !is_tty {
        return Err(TrebError::Cli(
            "no TTY detected; cannot display interactive multiselect".into(),
        ));
    }

    let items: Vec<String> = deployments
        .iter()
        .map(|d| format!("{}  {}  {}", d.id, d.contract_name, d.address))
        .collect();

    let selections =
        dialoguer::MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(prompt)
            .items(&items)
            .interact_on(&Term::stdout())
            .map_err(|e| TrebError::Cli(format!("multiselect error: {e}")))?;

    Ok(selections.into_iter().filter_map(|i| deployments.get(i)).collect())
}

/// Convenience wrapper that returns the ID of the selected deployment.
///
/// Returns `Ok(None)` when the list is empty or the user aborts.
/// Returns `Err(TrebError::Cli(...))` when not running in a TTY.
pub fn fuzzy_select_deployment_id(deployments: &[Deployment]) -> Result<Option<String>> {
    Ok(fuzzy_select_deployment(deployments, None)?.map(|d| d.id.clone()))
}

/// Score-based fuzzy filter: returns indices of items that match `query`.
fn fuzzy_filter(items: &[String], query: &str) -> Vec<usize> {
    use nucleo_matcher::{
        Matcher,
        pattern::{CaseMatching, Normalization, Pattern},
    };

    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let haystack = nucleo_matcher::Utf32String::from(item.as_str());
            let score = pattern.score(haystack.slice(..), &mut matcher);
            score.map(|_| i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };

    use super::*;

    fn make_deployment(id: &str, contract_name: &str, address: &str) -> Deployment {
        Deployment {
            id: id.into(),
            namespace: "default".into(),
            chain_id: 1,
            contract_name: contract_name.into(),
            label: "".into(),
            address: address.into(),
            deployment_type: DeploymentType::Singleton,
            transaction_id: "0x0".into(),
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
                path: "".into(),
                compiler_version: "".into(),
                bytecode_hash: "".into(),
                script_path: "".into(),
                git_commit: "".into(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn empty_list_returns_ok_none() {
        let result = fuzzy_select_deployment(&[], None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn non_tty_no_query_returns_cli_error() {
        // This test is designed to run in a non-TTY environment (CI / piped stdout).
        // If running in a real TTY, the test would try to open an interactive prompt,
        // so we skip it when stdout is a TTY.
        if console::Term::stdout().is_term() {
            return; // skip in interactive sessions
        }
        let d = make_deployment("id1", "Counter", "0xabc");
        let deployments = [d];
        let result = fuzzy_select_deployment(&deployments, None);
        assert!(matches!(result, Err(TrebError::Cli(_))));
    }

    #[test]
    fn fuzzy_select_network_empty_returns_ok_none() {
        let networks: Vec<String> = vec![];
        let result = fuzzy_select_network(&networks);
        assert!(matches!(result, Ok(None)));
    }
}
