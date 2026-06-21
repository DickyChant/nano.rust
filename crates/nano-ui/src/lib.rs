//! UI-facing session core and capability-gated front-ends for nano.rust.

pub mod plot;
pub mod session;

pub mod tui;

#[cfg(feature = "web")]
pub mod web;
