#[cfg(target_os = "windows")]
mod resolve;

#[cfg(target_os = "windows")]
pub use resolve::QtResolveTool;
