//! Shared output formatting utilities for CLI commands.
//!
//! Provides consistent output patterns: pretty JSON, UTF-8 tables with bold
//! headers, and aligned key-value pair printing.

use comfy_table::{Attribute, Cell, ContentArrangement, Table};
use serde::Serialize;

/// Print a value as pretty-printed JSON to stdout.
///
/// Recursively sorts all object keys so that output is deterministic
/// regardless of the underlying map type (e.g. `HashMap`).
pub fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    let json_value = sort_json_keys(serde_json::to_value(value)?);
    let json = serde_json::to_string_pretty(&json_value)?;
    println!("{json}");
    Ok(())
}

/// Recursively sort all object keys in a JSON value.
fn sort_json_keys(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
            let mut entries: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in entries {
                sorted.insert(k, sort_json_keys(v));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_json_keys).collect())
        }
        other => other,
    }
}

/// Build a UTF-8 table with bold headers.
pub fn build_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_preset(comfy_table::presets::UTF8_FULL);

    let header_cells: Vec<Cell> =
        headers.iter().map(|h| Cell::new(h).add_attribute(Attribute::Bold)).collect();
    table.set_header(header_cells);

    table
}

/// Print a table to stdout.
pub fn print_table(table: &Table) {
    println!("{table}");
}

/// Print key-value pairs with right-padded keys for alignment.
pub fn print_kv(pairs: &[(&str, &str)]) {
    let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (key, value) in pairs {
        println!("{:>width$}:  {}", key, value, width = max_key_len);
    }
}

/// Truncate an address to `0xABCD...EFGH` format (first 4 + last 4 hex chars).
pub fn truncate_address(address: &str) -> String {
    if address.len() >= 10 {
        format!("{}...{}", &address[..6], &address[address.len() - 4..])
    } else {
        address.to_string()
    }
}
