//! Shared output formatting utilities for CLI commands.
//!
//! Provides consistent output patterns: pretty JSON, UTF-8 tables with bold
//! headers, and aligned key-value pair printing.

use comfy_table::{Attribute, Cell, ContentArrangement, Table};
use serde::Serialize;

/// Print a value as pretty-printed JSON to stdout.
pub fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{json}");
    Ok(())
}

/// Build a UTF-8 table with bold headers.
pub fn build_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_preset(comfy_table::presets::UTF8_FULL);

    let header_cells: Vec<Cell> = headers
        .iter()
        .map(|h| Cell::new(h).add_attribute(Attribute::Bold))
        .collect();
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
