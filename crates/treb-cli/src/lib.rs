//! treb-cli library.
//!
//! The command modules are compiled into the library target so their unit tests
//! run under `cargo test -p treb-cli --lib`.

pub mod commands {
    pub mod resolve;
    pub mod run;
    pub mod sync;
    pub mod tag;
}
pub mod output;
pub mod ui;
