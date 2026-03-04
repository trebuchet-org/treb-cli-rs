//! Natspec parameter parser for `@custom:env` annotations.
//!
//! Parses `@custom:env` natspec annotations from compiled script artifacts
//! to discover what parameters a deployment script expects (e.g., environment
//! variables, addresses, deployments).

use serde_json::Value;

/// Supported parameter types for script parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterType {
    String,
    Address,
    Uint256,
    Int256,
    Bytes32,
    Bytes,
    Bool,
    Sender,
    Deployment,
    Artifact,
}

/// A script parameter extracted from `@custom:env` natspec annotations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptParameter {
    /// The parameter name.
    pub name: std::string::String,
    /// The parameter type.
    pub param_type: ParameterType,
    /// Human-readable description.
    pub description: std::string::String,
    /// Whether the parameter is optional.
    pub optional: bool,
}

/// Parse script parameters from devdoc JSON metadata.
///
/// Looks for `methods["run()"]["custom:env"]` in the devdoc JSON and
/// parses the annotation string. Returns an empty vec if the devdoc
/// is missing, has no `run()` method, or has no `custom:env` annotations.
pub fn parse_script_parameters(devdoc: &Value) -> Vec<ScriptParameter> {
    let env_str = devdoc
        .get("methods")
        .and_then(|m| m.get("run()"))
        .and_then(|r| r.get("custom:env"))
        .and_then(|v| v.as_str());

    match env_str {
        Some(s) => parse_custom_env_string(s),
        None => Vec::new(),
    }
}

/// Parse a `@custom:env` annotation string into script parameters.
///
/// Format: `{type[:optional]} name description text...`
///
/// Multiple parameters are concatenated in the same string, each starting
/// with `{type}`. The parser splits on `{` to find each parameter block.
pub fn parse_custom_env_string(env_str: &str) -> Vec<ScriptParameter> {
    let mut params = Vec::new();

    // Split on '{' to find each parameter block. The first split before any '{'
    // is ignored (it's empty or whitespace).
    for segment in env_str.split('{').skip(1) {
        // Find the closing '}' for the type specification
        let Some(close_brace) = segment.find('}') else {
            continue;
        };

        let type_spec = &segment[..close_brace];
        let rest = segment[close_brace + 1..].trim();

        // Parse type and optional modifier
        let (type_str, optional) = if let Some(base) = type_spec.strip_suffix(":optional") {
            (base, true)
        } else {
            (type_spec, false)
        };

        let Some(param_type) = parse_parameter_type(type_str) else {
            continue;
        };

        // Split rest into name and description (first word = name, rest = description)
        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = match parts.next() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let description = parts.next().unwrap_or("").trim().to_string();

        params.push(ScriptParameter { name, param_type, description, optional });
    }

    params
}

/// Map a type string to a `ParameterType` enum variant.
fn parse_parameter_type(s: &str) -> Option<ParameterType> {
    match s.to_lowercase().as_str() {
        "string" => Some(ParameterType::String),
        "address" => Some(ParameterType::Address),
        "uint256" => Some(ParameterType::Uint256),
        "int256" => Some(ParameterType::Int256),
        "bytes32" => Some(ParameterType::Bytes32),
        "bytes" => Some(ParameterType::Bytes),
        "bool" => Some(ParameterType::Bool),
        "sender" => Some(ParameterType::Sender),
        "deployment" => Some(ParameterType::Deployment),
        "artifact" => Some(ParameterType::Artifact),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_single_string_parameter() {
        let devdoc = json!({
            "methods": {
                "run()": {
                    "custom:env": "{string} label Deployment label"
                }
            }
        });

        let params = parse_script_parameters(&devdoc);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "label");
        assert_eq!(params[0].param_type, ParameterType::String);
        assert_eq!(params[0].description, "Deployment label");
        assert!(!params[0].optional);
    }

    #[test]
    fn parse_optional_address_parameter() {
        let devdoc = json!({
            "methods": {
                "run()": {
                    "custom:env": "{address:optional} owner Optional owner address"
                }
            }
        });

        let params = parse_script_parameters(&devdoc);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "owner");
        assert_eq!(params[0].param_type, ParameterType::Address);
        assert_eq!(params[0].description, "Optional owner address");
        assert!(params[0].optional);
    }

    #[test]
    fn parse_multi_parameter_string() {
        let devdoc = json!({
            "methods": {
                "run()": {
                    "custom:env": "{string} label Deployment label {address} owner Owner address {uint256:optional} threshold Voting threshold"
                }
            }
        });

        let params = parse_script_parameters(&devdoc);
        assert_eq!(params.len(), 3);

        assert_eq!(params[0].name, "label");
        assert_eq!(params[0].param_type, ParameterType::String);
        assert_eq!(params[0].description, "Deployment label");
        assert!(!params[0].optional);

        assert_eq!(params[1].name, "owner");
        assert_eq!(params[1].param_type, ParameterType::Address);
        assert_eq!(params[1].description, "Owner address");
        assert!(!params[1].optional);

        assert_eq!(params[2].name, "threshold");
        assert_eq!(params[2].param_type, ParameterType::Uint256);
        assert_eq!(params[2].description, "Voting threshold");
        assert!(params[2].optional);
    }

    #[test]
    fn empty_devdoc_returns_empty_vec() {
        let devdoc = json!({});
        let params = parse_script_parameters(&devdoc);
        assert!(params.is_empty());
    }

    #[test]
    fn missing_run_method_returns_empty_vec() {
        let devdoc = json!({
            "methods": {
                "setUp()": {
                    "custom:env": "{string} label Label"
                }
            }
        });

        let params = parse_script_parameters(&devdoc);
        assert!(params.is_empty());
    }

    #[test]
    fn missing_custom_env_returns_empty_vec() {
        let devdoc = json!({
            "methods": {
                "run()": {
                    "details": "Runs the deployment script"
                }
            }
        });

        let params = parse_script_parameters(&devdoc);
        assert!(params.is_empty());
    }

    #[test]
    fn all_parameter_types_supported() {
        let types = [
            ("string", ParameterType::String),
            ("address", ParameterType::Address),
            ("uint256", ParameterType::Uint256),
            ("int256", ParameterType::Int256),
            ("bytes32", ParameterType::Bytes32),
            ("bytes", ParameterType::Bytes),
            ("bool", ParameterType::Bool),
            ("sender", ParameterType::Sender),
            ("deployment", ParameterType::Deployment),
            ("artifact", ParameterType::Artifact),
        ];

        for (type_str, expected_type) in types {
            let env_str = format!("{{{type_str}}} param_{type_str} A {type_str} parameter");
            let params = parse_custom_env_string(&env_str);
            assert_eq!(params.len(), 1, "failed to parse type: {type_str}");
            assert_eq!(params[0].param_type, expected_type, "wrong type for: {type_str}");
            assert_eq!(params[0].name, format!("param_{type_str}"));
        }
    }

    #[test]
    fn parse_custom_env_string_directly() {
        let params = parse_custom_env_string("{sender} deployer The deployer sender");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "deployer");
        assert_eq!(params[0].param_type, ParameterType::Sender);
        assert_eq!(params[0].description, "The deployer sender");
        assert!(!params[0].optional);
    }

    #[test]
    fn unknown_type_is_skipped() {
        let params = parse_custom_env_string("{unknown} foo Bar {string} valid Valid param");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "valid");
        assert_eq!(params[0].param_type, ParameterType::String);
    }

    #[test]
    fn parameter_with_no_description() {
        let params = parse_custom_env_string("{address} owner");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "owner");
        assert_eq!(params[0].param_type, ParameterType::Address);
        assert_eq!(params[0].description, "");
        assert!(!params[0].optional);
    }
}
