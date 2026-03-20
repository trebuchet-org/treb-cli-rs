//! Terminal display state machine for broadcast output.
//!
//! Manages the interplay between status messages, result updates, and prompts.
//! All terminal writes go through this module to avoid races.
//!
//! Uses static status lines (no background spinner thread) to eliminate
//! ANSI escape race conditions between the spinner and output lines.

use std::io::Write;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use owo_colors::OwoColorize;

use crate::output;
use crate::ui::color;

/// Whether a status line is currently displayed.
enum DisplayState {
    /// No status line on screen.
    Idle,
    /// A static status message is on the current line (no newline).
    Status,
}

/// Manages terminal output during broadcast.
pub struct BroadcastDisplay {
    state: DisplayState,
    use_color: bool,
    quiet: bool,
}

impl BroadcastDisplay {
    pub fn new(quiet: bool) -> Self {
        Self {
            state: DisplayState::Idle,
            use_color: color::is_color_enabled(),
            quiet,
        }
    }

    /// Show a static status message (e.g. "Broadcasting...", "Simulating...").
    /// Overwrites any previous status line. No background thread.
    pub fn start_spinner(&mut self, msg: &str) {
        if self.quiet { return; }
        self.clear_line();
        if self.use_color {
            let _ = write!(std::io::stderr(), "  {}", msg.style(color::GRAY));
        } else {
            let _ = write!(std::io::stderr(), "  {msg}");
        }
        let _ = std::io::stderr().flush();
        self.state = DisplayState::Status;
    }

    /// Clear any status line and return to idle.
    pub fn stop(&mut self) {
        self.clear_line();
        self.state = DisplayState::Idle;
    }

    /// Alias for stop — clean terminal before a prompt.
    pub fn pause_for_prompt(&mut self) {
        self.stop();
    }

    /// Final cleanup.
    pub fn finish(&mut self) {
        self.stop();
    }

    // ── Events ──────────────────────────────────────────────────────────

    pub fn on_tx_executed(
        &self,
        sender_role: &str,
        tx_index: usize,
        hash: &str,
        block_number: u64,
        gas_used: u64,
    ) {
        if self.quiet { return; }
        let block = if block_number > 0 { format!("  block={block_number}") } else { String::new() };
        let gas = if gas_used > 0 { format!("  gas={gas_used}") } else { String::new() };
        if self.use_color {
            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                "executed".style(color::GREEN), sender_role.style(color::CYAN),
                tx_index, hash, block, gas);
        } else {
            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                "executed", sender_role, tx_index, hash, block, gas);
        }
    }

    pub fn on_governor_queued(
        &self, sender_role: &str, first_idx: usize, last_idx: usize, proposal_id: &str,
    ) {
        if self.quiet { return; }
        let range = if first_idx == last_idx { format!("{first_idx}") } else { format!("{first_idx}-{last_idx}") };
        if self.use_color {
            eprintln!("  {:<9} {:>10} idx={:<5}  proposal={}",
                "queued".style(color::YELLOW), sender_role.style(color::CYAN), range, proposal_id);
        } else {
            eprintln!("  {:<9} {:>10} idx={:<5}  proposal={}",
                "queued", sender_role, range, proposal_id);
        }
    }

    pub fn on_safe_queued(
        &self, sender_role: &str, first_idx: usize, last_idx: usize, safe_tx_hash: &str, nonce: u64,
    ) {
        if self.quiet { return; }
        let range = if first_idx == last_idx { format!("{first_idx}") } else { format!("{first_idx}-{last_idx}") };
        if self.use_color {
            eprintln!("  {:<9} {:>10} idx={:<5}  safe-hash={}  nonce={}",
                "queued".style(color::YELLOW), sender_role.style(color::CYAN), range, safe_tx_hash, nonce);
        } else {
            eprintln!("  {:<9} {:>10} idx={:<5}  safe-hash={}  nonce={}",
                "queued", sender_role, range, safe_tx_hash, nonce);
        }
    }

    pub fn on_tx_simulated(
        &self,
        sender_role: &str,
        tx_index: usize,
        hash: &str,
        block_number: u64,
        gas_used: u64,
    ) {
        if self.quiet { return; }
        let block = if block_number > 0 { format!("  block={block_number}") } else { String::new() };
        let gas = if gas_used > 0 { format!("  gas={gas_used}") } else { String::new() };
        if self.use_color {
            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                "simulated".style(color::GREEN), sender_role.style(color::CYAN),
                tx_index, hash, block, gas);
        } else {
            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                "simulated", sender_role, tx_index, hash, block, gas);
        }
    }

    // ── Build callback ──────────────────────────────────────────────────

    /// Build an `OnActionComplete` callback for use during broadcast.
    ///
    /// The callback clears the status line, prints the result, then
    /// shows a new status line for Broadcast results (not for queued,
    /// since those are followed by prompts).
    pub fn build_callback(&self) -> treb_forge::pipeline::OnActionComplete {
        let use_color = self.use_color;
        let global_offset = Arc::new(AtomicUsize::new(0));
        // Track whether a status line is showing so we know to clear it
        let has_status = Arc::new(std::sync::atomic::AtomicBool::new(true));

        Box::new(move |run, result| {
            // Clear status line if present
            if has_status.load(Ordering::Relaxed) {
                let _ = write!(std::io::stderr(), "\r\x1b[2K");
                let _ = std::io::stderr().flush();
                has_status.store(false, Ordering::Relaxed);
            }

            let offset = global_offset.load(Ordering::Relaxed);

            match result {
                treb_forge::pipeline::RunResult::Broadcast(receipts) => {
                    for (i, receipt) in receipts.iter().enumerate() {
                        let idx = offset + run.tx_indices.get(i).copied().unwrap_or(i);
                        let hash = format!("{:#x}", receipt.hash);
                        let block = if receipt.block_number > 0 { format!("  block={}", receipt.block_number) } else { String::new() };
                        let gas = if receipt.gas_used > 0 { format!("  gas={}", receipt.gas_used) } else { String::new() };
                        if use_color {
                            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                                "executed".style(color::GREEN), run.sender_role.style(color::CYAN),
                                idx, hash, block, gas);
                        } else {
                            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                                "executed", run.sender_role, idx, hash, block, gas);
                        }
                    }
                    global_offset.fetch_add(receipts.len(), Ordering::Relaxed);

                    // Show static status for next action
                    if use_color {
                        let _ = write!(std::io::stderr(), "  {}", "Broadcasting...".style(color::GRAY));
                    } else {
                        let _ = write!(std::io::stderr(), "  Broadcasting...");
                    }
                    let _ = std::io::stderr().flush();
                    has_status.store(true, Ordering::Relaxed);
                }
                treb_forge::pipeline::RunResult::GovernorProposed { proposal_id, tx_count, .. } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range = if first == last { format!("{first}") } else { format!("{first}-{last}") };
                    if use_color {
                        eprintln!("  {:<9} {:>10} idx={:<5}  proposal={}",
                            "queued".style(color::YELLOW), run.sender_role.style(color::CYAN), range, proposal_id);
                    } else {
                        eprintln!("  {:<9} {:>10} idx={:<5}  proposal={}",
                            "queued", run.sender_role, range, proposal_id);
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                    // No status line — prompt follows
                }
                treb_forge::pipeline::RunResult::SafeProposed { safe_tx_hash, nonce, tx_count, .. } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range = if first == last { format!("{first}") } else { format!("{first}-{last}") };
                    if use_color {
                        eprintln!("  {:<9} {:>10} idx={:<5}  safe-hash={:#x}  nonce={}",
                            "queued".style(color::YELLOW), run.sender_role.style(color::CYAN), range, safe_tx_hash, nonce);
                    } else {
                        eprintln!("  {:<9} {:>10} idx={:<5}  safe-hash={:#x}  nonce={}",
                            "queued", run.sender_role, range, safe_tx_hash, nonce);
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                    // No status line — prompt follows
                }
            }
        })
    }

    // ── Internal ────────────────────────────────────────────────────────

    fn clear_line(&self) {
        let _ = write!(std::io::stderr(), "\r\x1b[2K");
        let _ = std::io::stderr().flush();
    }
}

impl Drop for BroadcastDisplay {
    fn drop(&mut self) {
        if matches!(self.state, DisplayState::Status) {
            self.clear_line();
        }
    }
}
