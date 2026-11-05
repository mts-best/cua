#[cfg(target_os = "windows")]
pub mod uia;

#[cfg(target_os = "windows")]
pub use uia::{observe, resolve_window, ObserveError, ObserveOptions};
