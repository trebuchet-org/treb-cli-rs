//! Terminal display state machine for broadcast output.
//!
//! Manages the interplay between spinner, status updates, and prompts.
//! All terminal writes go through this module to avoid races between
//! the spinner thread and inline output.
//!
//! State transitions:
//!   Idle → Spinning(msg)        via start_spinner()
//!   Spinning → Idle             via stop()
//!   Spinning → WaitingForInput  via pause_for_prompt()
//!   WaitingForInput → Spinning  via resume_after_prompt()
//!   Any → Idle                  via finish()
//!
//! Events (on_tx_executed, on_tx_queued, etc.) automatically transition
//! from Spinning → print → back to Spinning (or Idle for queued).

use std::io::Write;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};

use owo_colors::OwoColorize;

use crate::output;
use crate::ui::{color, emoji};

/// Terminal state for broadcast display.
enum DisplayState {
    Idle,
    Spinning,
}

/// Manages terminal output during broadcast.
///
/// Owns the spinner lifecycle and ensures clean terminal state
/// before any output or prompt.
pub struct BroadcastDisplay {
    spinner: Arc<Mutex<Option<spinoff::Spinner>>>,
    state: DisplayState,
    global_tx_index: AtomicUsize,
    use_color: bool,
    quiet: bool,
}

impl BroadcastDisplay {
    /// Create a new display. Does not start the spinner.
    pub fn new(quiet: bool) -> Self {
        Self {
            spinner: Arc::new(Mutex::new(None)),
            state: DisplayState::Idle,
            global_tx_index: AtomicUsize::new(0),
            use_color: color::is_color_enabled(),
            quiet,
        }
    }

    /// Start showing a spinner with the given message.
    pub fn start_spinner(&mut self, msg: &str) {
        if self.quiet {
            return;
        }
        let mut guard = self.spinner.lock().unwrap();
        // Stop any existing spinner first
        if let Some(mut s) = guard.take() {
            s.clear();
        }
        *guard = Some(spinoff::Spinner::new(
            spinoff::spinners::Dots2,
            msg.to_string(),
            spinoff::Color::Cyan,
        ));
        self.state = DisplayState::Spinning;
    }

    /// Stop the spinner and clear its line. Terminal is clean after this.
    pub fn stop(&mut self) {
        self.clear_spinner();
        self.state = DisplayState::Idle;
    }

    /// Pause for a prompt — stop the spinner, ensure clean terminal.
    /// Call this BEFORE rendering any interactive prompt.
    pub fn pause_for_prompt(&mut self) {
        self.clear_spinner();
        // State stays Idle until resume_after_prompt
    }

    /// Resume the spinner after a prompt was answered.
    pub fn resume_spinner(&mut self, msg: &str) {
        self.start_spinner(msg);
    }

    /// Final cleanup — stop everything.
    pub fn finish(&mut self) {
        self.clear_spinner();
        self.state = DisplayState::Idle;
    }

    // ── Events ──────────────────────────────────────────────────────────

    /// A wallet transaction was executed on-chain.
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
            eprintln!("  {:<9} {:>10} [{:>2}]  tx={}{}{}",
                "executed".style(color::GREEN),
                sender_role.style(color::CYAN),
                tx_index,
                hash, block, gas,
            );
        } else {
            eprintln!("  {:<9} {:>10} [{:>2}]  tx={}{}{}",
                "executed", sender_role, tx_index, hash, block, gas);
        }
    }

    /// A governor proposal was queued (not yet executed).
    pub fn on_governor_queued(
        &self,
        sender_role: &str,
        first_idx: usize,
        last_idx: usize,
        proposal_id: &str,
    ) {
        if self.quiet { return; }

        let range = if first_idx == last_idx {
            format!("[{first_idx}]")
        } else {
            format!("[{first_idx}-{last_idx}]")
        };

        if self.use_color {
            eprintln!("  {:<9} {:>10} {:>5}  proposal={}",
                "queued".style(color::YELLOW),
                sender_role.style(color::CYAN),
                range,
                proposal_id,
            );
        } else {
            eprintln!("  {:<9} {:>10} {:>5}  proposal={}",
                "queued", sender_role, range, proposal_id);
        }
    }

    /// A Safe transaction was queued (proposed).
    pub fn on_safe_queued(
        &self,
        sender_role: &str,
        first_idx: usize,
        last_idx: usize,
        safe_tx_hash: &str,
        nonce: u64,
    ) {
        if self.quiet { return; }

        let range = if first_idx == last_idx {
            format!("[{first_idx}]")
        } else {
            format!("[{first_idx}-{last_idx}]")
        };

        if self.use_color {
            eprintln!("  {:<9} {:>10} {:>5}  safe-hash={}  nonce={}",
                "queued".style(color::YELLOW),
                sender_role.style(color::CYAN),
                range,
                safe_tx_hash, nonce,
            );
        } else {
            eprintln!("  {:<9} {:>10} {:>5}  safe-hash={}  nonce={}",
                "queued", sender_role, range, safe_tx_hash, nonce);
        }
    }

    /// A simulated transaction result (fork execution).
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
            eprintln!("  {:<9} {:>10} [{:>2}]  tx={}{}{}",
                "simulated".style(color::GREEN),
                sender_role.style(color::CYAN),
                tx_index,
                hash, block, gas,
            );
        } else {
            eprintln!("  {:<9} {:>10} [{:>2}]  tx={}{}{}",
                "simulated", sender_role, tx_index, hash, block, gas);
        }
    }

    // ── Global index tracking ───────────────────────────────────────────

    /// Get the current global tx index and advance by `count`.
    pub fn advance_index(&self, count: usize) -> usize {
        self.global_tx_index.fetch_add(count, Ordering::Relaxed)
    }

    /// Get the current global tx index without advancing.
    pub fn current_index(&self) -> usize {
        self.global_tx_index.load(Ordering::Relaxed)
    }

    // ── Build callback ──────────────────────────────────────────────────

    /// Build an `OnActionComplete` callback that routes events through this display.
    ///
    /// The callback stops the spinner, prints the appropriate line, then
    /// restarts the spinner for Broadcast results (not for queued results,
    /// since those are followed by prompts).
    pub fn build_callback(&self) -> treb_forge::pipeline::OnActionComplete {
        let spinner = self.spinner.clone();
        let use_color = self.use_color;
        let global_offset = Arc::new(AtomicUsize::new(0));

        Box::new(move |run, result| {
            // Stop spinner and clear line
            {
                let mut guard = spinner.lock().unwrap();
                if let Some(mut s) = guard.take() {
                    s.clear();
                }
            }
            let _ = write!(std::io::stderr(), "\r\x1b[2K");
            let _ = std::io::stderr().flush();

            let offset = global_offset.load(Ordering::Relaxed);

            match result {
                treb_forge::pipeline::RunResult::Broadcast(receipts) => {
                    for (i, receipt) in receipts.iter().enumerate() {
                        let global_idx = offset + run.tx_indices.get(i).copied().unwrap_or(i);
                        let hash = format!("{:#x}", receipt.hash);
                        let block = if receipt.block_number > 0 { format!("  block={}", receipt.block_number) } else { String::new() };
                        let gas = if receipt.gas_used > 0 { format!("  gas={}", receipt.gas_used) } else { String::new() };
                        if use_color {
                            eprintln!("  {:<9} {:>10} [{:>2}]  tx={}{}{}",
                                "executed".style(color::GREEN),
                                run.sender_role.style(color::CYAN),
                                global_idx, hash, block, gas,
                            );
                        } else {
                            eprintln!("  {:<9} {:>10} [{:>2}]  tx={}{}{}",
                                "executed", run.sender_role, global_idx, hash, block, gas);
                        }
                    }
                    global_offset.fetch_add(receipts.len(), Ordering::Relaxed);

                    // Restart spinner — more broadcasts may follow
                    *spinner.lock().unwrap() = Some(spinoff::Spinner::new(
                        spinoff::spinners::Dots2,
                        "Broadcasting".to_string(),
                        spinoff::Color::Cyan,
                    ));
                }
                treb_forge::pipeline::RunResult::GovernorProposed { proposal_id, tx_count, .. } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range = if first == last { format!("[{first}]") } else { format!("[{first}-{last}]") };
                    if use_color {
                        eprintln!("  {:<9} {:>10} {:>5}  proposal={}",
                            "queued".style(color::YELLOW),
                            run.sender_role.style(color::CYAN),
                            range, proposal_id,
                        );
                    } else {
                        eprintln!("  {:<9} {:>10} {:>5}  proposal={}",
                            "queued", run.sender_role, range, proposal_id);
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                    // Do NOT restart spinner — a prompt follows
                }
                treb_forge::pipeline::RunResult::SafeProposed { safe_tx_hash, nonce, tx_count, .. } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range = if first == last { format!("[{first}]") } else { format!("[{first}-{last}]") };
                    if use_color {
                        eprintln!("  {:<9} {:>10} {:>5}  safe-hash={:#x}  nonce={}",
                            "queued".style(color::YELLOW),
                            run.sender_role.style(color::CYAN),
                            range, safe_tx_hash, nonce,
                        );
                    } else {
                        eprintln!("  {:<9} {:>10} {:>5}  safe-hash={:#x}  nonce={}",
                            "queued", run.sender_role, range, safe_tx_hash, nonce);
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                    // Do NOT restart spinner — a prompt follows
                }
            }
        })
    }

    // ── Internal ────────────────────────────────────────────────────────

    fn clear_spinner(&mut self) {
        let mut guard = self.spinner.lock().unwrap();
        if let Some(mut s) = guard.take() {
            s.clear();
        }
        drop(guard);
        let _ = write!(std::io::stderr(), "\r\x1b[2K");
        let _ = std::io::stderr().flush();
    }
}

impl Drop for BroadcastDisplay {
    fn drop(&mut self) {
        self.finish();
    }
}
