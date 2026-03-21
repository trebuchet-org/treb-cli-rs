//! Terminal display state machine for broadcast output.
//!
//! Manages the interplay between spinner animation, result updates, and prompts.
//! Uses a background spinner thread with a shared output lock so the callback
//! can pause the animation, print result lines, and resume cleanly.
//!
//! The spinner uses the same Dots2 style and cyan color as `spinoff::Spinner`
//! elsewhere in the CLI, but adds pause/resume support needed by the broadcast
//! callback.

use std::io::Write;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;

use owo_colors::OwoColorize;

use crate::ui::color;

/// Dots2 frames — matches `spinoff::spinners::Dots2` used elsewhere in the CLI.
const SPINNER_FRAMES: &[&str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

/// Manages terminal output during broadcast.
///
/// Wraps a pausable background spinner that coordinates with the broadcast
/// callback via a shared output lock. We can't use `spinoff::Spinner` directly
/// because it doesn't support pause/resume — every `stop()` kills the thread.
pub struct BroadcastDisplay {
    use_color: bool,
    quiet: bool,
    /// Shared flag: when true the spinner thread animates.
    spinner_active: Arc<AtomicBool>,
    /// Shared flag: when true the spinner thread exits.
    spinner_stop: Arc<AtomicBool>,
    /// Background spinner thread handle.
    spinner_thread: Option<std::thread::JoinHandle<()>>,
    /// Mutex held by both the spinner and the callback for stderr writes.
    output_lock: Arc<Mutex<()>>,
}

impl BroadcastDisplay {
    pub fn new(quiet: bool) -> Self {
        Self {
            use_color: color::is_color_enabled(),
            quiet,
            spinner_active: Arc::new(AtomicBool::new(false)),
            spinner_stop: Arc::new(AtomicBool::new(false)),
            spinner_thread: None,
            output_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Start an animated spinner with the given message on a background thread.
    /// Uses the same Dots2 frames and cyan color as `spinoff::Spinner`.
    pub fn start_spinner(&mut self, msg: &str) {
        if self.quiet { return; }
        self.stop_spinner_thread();

        let msg = msg.to_string();
        let active = self.spinner_active.clone();
        let stop = self.spinner_stop.clone();
        let lock = self.output_lock.clone();
        let use_color = self.use_color;

        active.store(true, Ordering::Relaxed);
        stop.store(false, Ordering::Relaxed);

        self.spinner_thread = Some(std::thread::spawn(move || {
            let mut i = 0usize;
            while !stop.load(Ordering::Relaxed) {
                if active.load(Ordering::Relaxed) {
                    if let Ok(_guard) = lock.lock() {
                        if active.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                            let frame = SPINNER_FRAMES[i % SPINNER_FRAMES.len()];
                            if use_color {
                                let _ = write!(
                                    std::io::stderr(),
                                    "\r\x1b[2K{} {}",
                                    frame.style(color::CYAN),
                                    msg.style(color::GRAY),
                                );
                            } else {
                                let _ = write!(std::io::stderr(), "\r\x1b[2K{frame} {msg}");
                            }
                            let _ = std::io::stderr().flush();
                        }
                    }
                }
                i = i.wrapping_add(1);
                std::thread::sleep(SPINNER_INTERVAL);
            }
            if let Ok(_guard) = lock.lock() {
                let _ = write!(std::io::stderr(), "\r\x1b[2K");
                let _ = std::io::stderr().flush();
            }
        }));
    }

    /// Clear spinner and return to idle.
    pub fn stop(&mut self) {
        self.stop_spinner_thread();
    }

    /// Alias for stop — clean terminal before a prompt.
    pub fn pause_for_prompt(&mut self) {
        self.stop();
    }

    /// Final cleanup.
    pub fn finish(&mut self) {
        self.stop();
    }

    fn stop_spinner_thread(&mut self) {
        self.spinner_active.store(false, Ordering::Relaxed);
        self.spinner_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.spinner_thread.take() {
            let _ = t.join();
        }
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
        let idx = format!("{tx_index}");
        self.print_result_line("executed", color::GREEN, sender_role, &idx, hash, block_number, gas_used);
    }

    pub fn on_governor_queued(
        &self, sender_role: &str, first_idx: usize, last_idx: usize, proposal_id: &str,
    ) {
        if self.quiet { return; }
        let range = if first_idx == last_idx { format!("{first_idx}") } else { format!("{first_idx}-{last_idx}") };
        if self.use_color {
            eprintln!("  {:<9} {:>10} {}={:<5}  {}={}",
                "queued".style(color::YELLOW), sender_role.style(color::CYAN),
                "idx".style(color::MUTED), range, "proposal".style(color::MUTED), proposal_id);
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
            eprintln!("  {:<9} {:>10} {}={:<5}  {}={}  {}={}",
                "queued".style(color::YELLOW), sender_role.style(color::CYAN),
                "idx".style(color::MUTED), range,
                "safe-hash".style(color::MUTED), safe_tx_hash,
                "nonce".style(color::MUTED), nonce);
        } else {
            eprintln!("  {:<9} {:>10} idx={:<5}  safe-hash={}  nonce={}",
                "queued", sender_role, range, safe_tx_hash, nonce);
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
        if self.quiet { return; }
        self.print_result_line("simulated", color::GREEN, sender_role, idx_range, hash, block_number, gas_used);
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
        let block = if block_number > 0 { format!("  {}={block_number}", if self.use_color { format!("{}", "block".style(color::MUTED)) } else { "block".to_string() }) } else { String::new() };
        let gas = if gas_used > 0 { format!("  {}={gas_used}", if self.use_color { format!("{}", "gas".style(color::MUTED)) } else { "gas".to_string() }) } else { String::new() };
        if self.use_color {
            eprintln!("  {:<9} {:>10} {}={:<5}  {}={}{}{}",
                label.style(label_style), sender_role.style(color::CYAN),
                "idx".style(color::MUTED), idx_range,
                "tx".style(color::MUTED), hash,
                block, gas);
        } else {
            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                label, sender_role, idx_range, hash, block, gas);
        }
    }

    // ── Build callback ──────────────────────────────────────────────────

    /// Build an `OnActionComplete` callback for use during broadcast.
    ///
    /// The callback pauses the spinner, prints the result, then resumes
    /// the spinner for Broadcast results (not for queued, since prompts follow).
    pub fn build_callback(&self) -> treb_forge::pipeline::OnActionComplete {
        let use_color = self.use_color;
        let global_offset = Arc::new(AtomicUsize::new(0));
        let spinner_active = self.spinner_active.clone();
        let output_lock = self.output_lock.clone();

        Box::new(move |run, result| {
            // Pause spinner and take output lock
            spinner_active.store(false, Ordering::Relaxed);
            let _guard = output_lock.lock().unwrap();

            // Clear spinner line
            let _ = write!(std::io::stderr(), "\r\x1b[2K");
            let _ = std::io::stderr().flush();

            let offset = global_offset.load(Ordering::Relaxed);

            match result {
                treb_forge::pipeline::RunResult::Broadcast(receipts) => {
                    for (i, receipt) in receipts.iter().enumerate() {
                        let idx = offset + run.tx_indices.get(i).copied().unwrap_or(i);
                        let hash = format!("{:#x}", receipt.hash);
                        let block = if receipt.block_number > 0 { format!("  {}={}", if use_color { format!("{}", "block".style(color::MUTED)) } else { "block".into() }, receipt.block_number) } else { String::new() };
                        let gas = if receipt.gas_used > 0 { format!("  {}={}", if use_color { format!("{}", "gas".style(color::MUTED)) } else { "gas".into() }, receipt.gas_used) } else { String::new() };
                        if use_color {
                            eprintln!("  {:<9} {:>10} {}={:<5}  {}={}{}{}",
                                "executed".style(color::GREEN), run.sender_role.style(color::CYAN),
                                "idx".style(color::MUTED), idx,
                                "tx".style(color::MUTED), hash, block, gas);
                        } else {
                            eprintln!("  {:<9} {:>10} idx={:<5}  tx={}{}{}",
                                "executed", run.sender_role, idx, hash, block, gas);
                        }
                    }
                    global_offset.fetch_add(receipts.len(), Ordering::Relaxed);

                    // Resume spinner for next action
                    spinner_active.store(true, Ordering::Relaxed);
                }
                treb_forge::pipeline::RunResult::GovernorProposed { proposal_id, tx_count, .. } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range = if first == last { format!("{first}") } else { format!("{first}-{last}") };
                    if use_color {
                        eprintln!("  {:<9} {:>10} {}={:<5}  {}={}",
                            "queued".style(color::YELLOW), run.sender_role.style(color::CYAN),
                            "idx".style(color::MUTED), range,
                            "proposal".style(color::MUTED), proposal_id);
                    } else {
                        eprintln!("  {:<9} {:>10} idx={:<5}  proposal={}",
                            "queued", run.sender_role, range, proposal_id);
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                    // Keep spinner paused — prompt may follow
                }
                treb_forge::pipeline::RunResult::SafeProposed { safe_tx_hash, nonce, tx_count, .. } => {
                    let first = offset;
                    let last = offset + tx_count.saturating_sub(1);
                    let range = if first == last { format!("{first}") } else { format!("{first}-{last}") };
                    if use_color {
                        eprintln!("  {:<9} {:>10} {}={:<5}  {}={:#x}  {}={}",
                            "queued".style(color::YELLOW), run.sender_role.style(color::CYAN),
                            "idx".style(color::MUTED), range,
                            "safe-hash".style(color::MUTED), safe_tx_hash,
                            "nonce".style(color::MUTED), nonce);
                    } else {
                        eprintln!("  {:<9} {:>10} idx={:<5}  safe-hash={:#x}  nonce={}",
                            "queued", run.sender_role, range, safe_tx_hash, nonce);
                    }
                    global_offset.fetch_add(*tx_count, Ordering::Relaxed);
                    // Keep spinner paused — prompt may follow
                }
            }
        })
    }
}

impl Drop for BroadcastDisplay {
    fn drop(&mut self) {
        self.stop_spinner_thread();
    }
}
