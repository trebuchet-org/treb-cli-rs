//! Terminal display for broadcast output.
//!
//! Wraps `indicatif::ProgressBar` to coordinate spinner animation with
//! inline result output during transaction broadcasting. Uses `pb.println()`
//! to print result lines above the spinner and `pb.suspend()` for prompts.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use owo_colors::OwoColorize;

use crate::ui::color;

/// Manages terminal output during broadcast.
///
/// Wraps an `indicatif::ProgressBar` spinner. The broadcast callback uses
/// `pb.println()` to print result lines above the spinner without stopping
/// the animation.
pub struct BroadcastDisplay {
    spinner: indicatif::ProgressBar,
    use_color: bool,
    quiet: bool,
}

impl BroadcastDisplay {
    pub fn new(quiet: bool) -> Self {
        let spinner = if quiet {
            indicatif::ProgressBar::hidden()
        } else {
            super::spinner::create_spinner("")
        };
        // Don't tick yet — wait for start_spinner()
        spinner.disable_steady_tick();

        Self { spinner, use_color: color::is_color_enabled(), quiet }
    }

    /// Start the spinner with the given message.
    pub fn start_spinner(&self, msg: &str) {
        if self.quiet {
            return;
        }
        self.spinner.set_message(msg.to_string());
        self.spinner.enable_steady_tick(std::time::Duration::from_millis(80));
    }

    /// Stop the spinner and clear its line.
    pub fn stop(&self) {
        self.spinner.finish_and_clear();
    }

    /// Alias for stop — clean terminal before a prompt.
    pub fn pause_for_prompt(&self) {
        self.stop();
    }

    /// Final cleanup.
    pub fn finish(&self) {
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
        if self.quiet {
            return;
        }
        let idx = format!("{tx_index}");
        self.print_result_line(
            "executed",
            color::GREEN,
            sender_role,
            &idx,
            hash,
            block_number,
            gas_used,
        );
    }

    pub fn on_governor_queued(
        &self,
        sender_role: &str,
        first_idx: usize,
        last_idx: usize,
        proposal_id: &str,
    ) {
        if self.quiet {
            return;
        }
        let range = if first_idx == last_idx {
            format!("{first_idx}")
        } else {
            format!("{first_idx}-{last_idx}")
        };
        if self.use_color {
            self.spinner.println(format!(
                "  {:<9} {:>10} {}={:<5}  {}={}",
                "queued".style(color::YELLOW),
                sender_role.style(color::CYAN),
                "idx".style(color::MUTED),
                range,
                "proposal".style(color::MUTED),
                proposal_id
            ));
        } else {
            self.spinner.println(format!(
                "  {:<9} {:>10} idx={:<5}  proposal={}",
                "queued", sender_role, range, proposal_id
            ));
        }
    }

    pub fn on_safe_queued(
        &self,
        sender_role: &str,
        first_idx: usize,
        last_idx: usize,
        safe_tx_hash: &str,
        nonce: u64,
    ) {
        if self.quiet {
            return;
        }
        let range = if first_idx == last_idx {
            format!("{first_idx}")
        } else {
            format!("{first_idx}-{last_idx}")
        };
        if self.use_color {
            self.spinner.println(format!(
                "  {:<9} {:>10} {}={:<5}  {}={}  {}={}",
                "queued".style(color::YELLOW),
                sender_role.style(color::CYAN),
                "idx".style(color::MUTED),
                range,
                "safe-hash".style(color::MUTED),
                safe_tx_hash,
                "nonce".style(color::MUTED),
                nonce
            ));
        } else {
            self.spinner.println(format!(
                "  {:<9} {:>10} idx={:<5}  safe-hash={}  nonce={}",
                "queued", sender_role, range, safe_tx_hash, nonce
            ));
        }
    }

    pub fn on_tx_simulated(
        &self,
        sender_role: &str,
        idx_range: &str,
        hash: &str,
        block_number: u64,
        gas_used: u64,
    ) {
        if self.quiet {
            return;
        }
        self.print_result_line(
            "simulated",
            color::GREEN,
            sender_role,
            idx_range,
            hash,
            block_number,
            gas_used,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn print_result_line(
        &self,
        label: &str,
        label_style: owo_colors::Style,
        sender_role: &str,
        idx_range: &str,
        hash: &str,
        block_number: u64,
        gas_used: u64,
    ) {
        let block = if block_number > 0 {
            format!(
                "  {}={block_number}",
                if self.use_color {
                    format!("{}", "block".style(color::MUTED))
                } else {
                    "block".to_string()
                }
            )
        } else {
            String::new()
        };
        let gas = if gas_used > 0 {
            format!(
                "  {}={gas_used}",
                if self.use_color {
                    format!("{}", "gas".style(color::MUTED))
                } else {
                    "gas".to_string()
                }
            )
        } else {
            String::new()
        };
        if self.use_color {
            self.spinner.println(format!(
                "  {:<9} {:>10} {}={:<5}  {}={}{}{}",
                label.style(label_style),
                sender_role.style(color::CYAN),
                "idx".style(color::MUTED),
                idx_range,
                "tx".style(color::MUTED),
                hash,
                block,
                gas
            ));
        } else {
            self.spinner.println(format!(
                "  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                label, sender_role, idx_range, hash, block, gas
            ));
        }
    }

    // ── Build callback ──────────────────────────────────────────────────

    /// Build an `OnActionComplete` callback for use during broadcast.
    ///
    /// Uses `spinner.println()` to print result lines above the spinner
    /// without stopping the animation.
    pub fn build_callback(&self) -> treb_forge::pipeline::OnActionComplete {
        let use_color = self.use_color;
        let global_offset = Arc::new(AtomicUsize::new(0));
        let spinner = self.spinner.clone();

        Box::new(move |run, result| {
            let offset = global_offset.load(Ordering::Relaxed);

            match result {
                treb_forge::pipeline::RunResult::Broadcast(receipts) => {
                    for (i, receipt) in receipts.iter().enumerate() {
                        let idx = offset + run.tx_indices.get(i).copied().unwrap_or(i);
                        let hash = format!("{:#x}", receipt.hash);
                        let block = if receipt.block_number > 0 {
                            format!(
                                "  {}={}",
                                if use_color {
                                    format!("{}", "block".style(color::MUTED))
                                } else {
                                    "block".into()
                                },
                                receipt.block_number
                            )
                        } else {
                            String::new()
                        };
                        let gas = if receipt.gas_used > 0 {
                            format!(
                                "  {}={}",
                                if use_color {
                                    format!("{}", "gas".style(color::MUTED))
                                } else {
                                    "gas".into()
                                },
                                receipt.gas_used
                            )
                        } else {
                            String::new()
                        };
                        if use_color {
                            spinner.println(format!(
                                "  {:<9} {:>10} {}={:<5}  {}={}{}{}",
                                "executed".style(color::GREEN),
                                run.sender_role.style(color::CYAN),
                                "idx".style(color::MUTED),
                                idx,
                                "tx".style(color::MUTED),
                                hash,
                                block,
                                gas
                            ));
                        } else {
                            spinner.println(format!(
                                "  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                                "executed", run.sender_role, idx, hash, block, gas
                            ));
                        }
                    }
                    global_offset.fetch_add(receipts.len(), Ordering::Relaxed);
                }
                treb_forge::pipeline::RunResult::GovernorProposed {
                    proposal_id, tx_count, ..
                } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range =
                        if first == last { format!("{first}") } else { format!("{first}-{last}") };
                    if use_color {
                        spinner.println(format!(
                            "  {:<9} {:>10} {}={:<5}  {}={}",
                            "queued".style(color::YELLOW),
                            run.sender_role.style(color::CYAN),
                            "idx".style(color::MUTED),
                            range,
                            "proposal".style(color::MUTED),
                            proposal_id
                        ));
                    } else {
                        spinner.println(format!(
                            "  {:<9} {:>10} idx={:<5}  proposal={}",
                            "queued", run.sender_role, range, proposal_id
                        ));
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                }
                treb_forge::pipeline::RunResult::SafeProposed {
                    safe_tx_hash,
                    nonce,
                    tx_count,
                    ..
                } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range =
                        if first == last { format!("{first}") } else { format!("{first}-{last}") };
                    if use_color {
                        spinner.println(format!(
                            "  {:<9} {:>10} {}={:<5}  {}={:#x}  {}={}",
                            "queued".style(color::YELLOW),
                            run.sender_role.style(color::CYAN),
                            "idx".style(color::MUTED),
                            range,
                            "safe-hash".style(color::MUTED),
                            safe_tx_hash,
                            "nonce".style(color::MUTED),
                            nonce
                        ));
                    } else {
                        spinner.println(format!(
                            "  {:<9} {:>10} idx={:<5}  safe-hash={:#x}  nonce={}",
                            "queued", run.sender_role, range, safe_tx_hash, nonce
                        ));
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                }
            }
        })
    }
}
