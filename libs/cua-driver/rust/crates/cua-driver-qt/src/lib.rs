//! Qt test-agent tools layered on cua-driver.
//!
//! The crate owns the normalized observation model (window-local logical
//! rectangles, stable fingerprints) and registers `qt_*` tools into the
//! driver's host-tool hook. It depends on `platform-windows` public APIs but
//! nothing in `platform-windows` depends on it, so the dependency graph stays
//! acyclic.

pub mod coord;
pub mod identity;
pub mod model;
pub mod selector;

pub mod adapter;

pub mod tools;

use cua_driver_core::tool::ToolRegistry;

/// Register every `qt_*` tool. Called from the driver host's
/// `register_host_tools` hook, so CLI `call`, MCP and `tools` metadata all see
/// the same inventory.
pub fn register_qt_tools(registry: &mut ToolRegistry) {
    #[cfg(target_os = "windows")]
    {
        registry.register(Box::new(tools::QtResolveTool));
    }
    #[cfg(not(target_os = "windows"))]
    let _ = registry;
}

#[cfg(target_os = "windows")]
mod runtime {
    use platform_windows::uia::snapshot::Snapshots;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    static SNAPSHOTS: OnceLock<Arc<Snapshots>> = OnceLock::new();
    static REVISIONS: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();

    pub(crate) fn snapshots() -> &'static Arc<Snapshots> {
        SNAPSHOTS.get_or_init(|| Arc::new(Snapshots::new()))
    }

    /// Monotonic per-window observation counter. Revision lets a caller tell
    /// two observations apart even when both carry the same node set.
    pub(crate) fn next_revision(window_id: u64) -> u64 {
        let mut revisions = REVISIONS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let revision = revisions.entry(window_id).or_insert(0);
        *revision += 1;
        *revision
    }
}

#[cfg(target_os = "windows")]
pub(crate) use runtime::{next_revision, snapshots};

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn registration_adds_qt_resolve_to_the_shared_registry() {
        let mut registry = ToolRegistry::new();
        register_qt_tools(&mut registry);
        let inventory = registry.tools_list();
        let names: Vec<&str> = inventory["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"qt_resolve"), "{names:?}");
    }
}
