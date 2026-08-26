//! svnui library crate — an SVN TUI client inspired by gitui.
//!
//! The binary (`src/main.rs`) is a thin wrapper around this library so that
//! benchmarks (`benches/`) and integration tests can reuse the code.

pub mod app;
pub mod components;
pub mod keys;
pub mod popups;
pub mod queue;
pub mod status;
pub mod strings;
pub mod svn;
pub mod test_support;
pub mod ui;
