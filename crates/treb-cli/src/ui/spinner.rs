//! Shared spinner helper wrapping `indicatif::ProgressBar`.
//!
//! Provides a consistent Dots2-style cyan spinner across the CLI.
//! The returned `ProgressBar` supports `println()` (print above the spinner),
//! `suspend(|| { ... })` (pause, run closure, resume), and `set_message()`
//! for updating the label — all without killing the animation thread.

use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

/// Dots2 frames — matches the spinoff::spinners::Dots2 style previously used.
const DOTS2_FRAMES: &[&str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷", " "];

/// Create an animated spinner on stderr with the given message.
///
/// The spinner uses Dots2 braille frames in cyan, consistent with all
/// other spinner sites in the CLI. Call `.finish_and_clear()` to remove it.
pub fn create_spinner(msg: impl Into<std::borrow::Cow<'static, str>>) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(DOTS2_FRAMES)
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message(msg);
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}
