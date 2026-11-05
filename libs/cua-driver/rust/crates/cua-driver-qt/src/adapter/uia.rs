use crate::coord;
use crate::model::{Node, Observation, Rect, Source, WindowRef};
use cua_driver_core::element_token::{format_snapshot_id, token_for};
use platform_windows::uia::snapshot::{SnapshotKind, UiaSnapshot};
use platform_windows::uia::{walk_tree_bounded, UiaNode};
use platform_windows::win32::{list_windows, WindowInfo};
use std::collections::{BTreeMap, HashMap};

const OUT_OF_BOUNDS_TOLERANCE: f64 = 2.0;

#[derive(Clone, Copy, Debug)]
pub struct ObserveOptions {
    pub max_elements: usize,
    pub max_depth: usize,
}

impl Default for ObserveOptions {
    fn default() -> Self {
        Self {
            max_elements: 5000,
            max_depth: 25,
        }
    }
}

#[derive(Debug)]
pub struct ObserveError {
    pub code: &'static str,
    pub message: String,
}

impl ObserveError {
    fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }
}

impl std::fmt::Display for ObserveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Pick the window a call means: an explicit handle, or the pid's largest
/// on-screen, non-minimized top-level window. Never guesses across processes.
pub fn resolve_window(
    pid: Option<u32>,
    window_id: Option<u64>,
) -> Result<WindowInfo, ObserveError> {
    let windows = list_windows(pid);
    let known = || {
        windows
            .iter()
            .map(|window| format!("{} (pid {})", window.hwnd, window.pid))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let window = match window_id {
        Some(window_id) => windows
            .iter()
            .find(|window| window.hwnd == window_id)
            .cloned()
            .ok_or_else(|| {
                ObserveError::new(
                    "window_not_found",
                    format!("window {window_id} not found; known windows: [{}]", known()),
                )
            })?,
        None => windows
            .iter()
            .filter(|window| window.is_on_screen && !window.minimized)
            .max_by_key(|window| i64::from(window.width) * i64::from(window.height))
            .cloned()
            .ok_or_else(|| {
                ObserveError::new(
                    "no_target_window",
                    format!("no on-screen window to observe; known windows: [{}]", known()),
                )
            })?,
    };

    // A minimized window reports the icon-frame rectangle while UIA keeps
    // reporting the restored geometry, so every rectangle would be out of
    // bounds. Refuse instead of returning a coordinate space that is not real.
    if window.minimized {
        return Err(ObserveError::new(
            "window_minimized",
            format!(
                "window {} (pid {}) is minimized; restore it before observing",
                window.hwnd, window.pid
            ),
        ));
    }

    Ok(window)
}

struct Normalized {
    nodes: Vec<Node>,
    by_id: HashMap<String, usize>,
    warnings: Vec<String>,
    degraded: Option<String>,
}

/// Walk the window's UIA tree, publish it into the qt observation store, and
/// normalize every node into window-local logical coordinates.
pub fn observe(window: &WindowInfo, options: ObserveOptions) -> Result<Observation, ObserveError> {
    let scale = coord::window_scale(window.hwnd);
    let tree = walk_tree_bounded(window.hwnd, None, options.max_elements, options.max_depth);
    if tree.nodes.is_empty() && tree.tree_markdown.starts_with("UIA init failed") {
        return Err(ObserveError::new("uia_unavailable", tree.tree_markdown));
    }

    let snapshot_handle = crate::snapshots().publish(
        window.pid as i32,
        window.hwnd,
        UiaSnapshot::from_nodes(&tree.nodes, SnapshotKind::Uia),
    );
    let snapshot_id = format_snapshot_id(snapshot_handle);

    let normalized = normalize(&tree.nodes, window, scale, snapshot_handle);

    let revision = crate::next_revision(window.hwnd);
    let source_counts = BTreeMap::from([(Source::Uia, normalized.nodes.len())]);

    Ok(Observation {
        snapshot_id,
        window: WindowRef {
            window_id: window.hwnd,
            pid: window.pid,
            title: window.title.clone(),
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
            scale,
        },
        revision,
        elements_complete: tree.elements_complete,
        nodes: normalized.nodes,
        by_id: normalized.by_id,
        warnings: normalized.warnings,
        degraded: normalized.degraded,
        source_counts,
    })
}

fn normalize(
    nodes: &[UiaNode],
    window: &WindowInfo,
    scale: f64,
    snapshot_handle: u32,
) -> Normalized {
    let bounds = coord::window_local_bounds(window.width, window.height, scale);
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut by_id = HashMap::with_capacity(nodes.len());
    let mut warnings = Vec::new();
    let mut degraded = None;

    for (walk_index, node) in nodes.iter().enumerate() {
        // Actionable nodes keep the upstream handle as their observation id;
        // content-only nodes (no cached UIA pattern) get a walk-local id and
        // no token, so an act step can never mistake them for invokable.
        let (node_id, element_token) = match node.element_index {
            Some(index) => (
                format!("n{index}"),
                Some(token_for(snapshot_handle, index)),
            ),
            None => (format!("u{walk_index}"), None),
        };
        let rect = match node.rect {
            Some(rect) => coord::screen_rect_to_window_local(rect, window.x, window.y, scale),
            None => Rect::default(),
        };
        if !rect.is_empty()
            && (rect.x < bounds.x - OUT_OF_BOUNDS_TOLERANCE
                || rect.y < bounds.y - OUT_OF_BOUNDS_TOLERANCE
                || rect.right() > bounds.w + OUT_OF_BOUNDS_TOLERANCE
                || rect.bottom() > bounds.h + OUT_OF_BOUNDS_TOLERANCE)
        {
            warnings.push(format!(
                "dropped out-of-bounds node {node_id} {} ({:.0},{:.0} {:.0}x{:.0})",
                node.control_type, rect.x, rect.y, rect.w, rect.h
            ));
            degraded.get_or_insert_with(|| "coordinate_out_of_bounds".to_owned());
            continue;
        }

        let mut states = BTreeMap::new();
        if let Some(selected) = node.selected {
            states.insert("selected".to_owned(), selected.to_string());
        }
        let mut attrs = BTreeMap::new();
        if let Some(value) = node.value.clone().filter(|value| !value.is_empty()) {
            attrs.insert("value".to_owned(), value);
        }
        if node.msaa_role.is_some() {
            attrs.insert("channel".to_owned(), "msaa".to_owned());
        }

        by_id.insert(node_id.clone(), out.len());
        out.push(Node {
            node_id,
            element_token,
            parent_id: node.parent_element_index.map(|parent| format!("n{parent}")),
            role: node.control_type.clone(),
            name: node.name.clone(),
            automation_id: node.automation_id.clone(),
            class_name: None,
            object_name: None,
            rect,
            visible: !rect.is_empty(),
            enabled: node.enabled.unwrap_or(true),
            states,
            actions: node.actions.clone(),
            source: Source::Uia,
            attrs,
            children: Vec::new(),
        });
    }

    let links: Vec<(String, String)> = out
        .iter()
        .filter_map(|node| node.parent_id.clone().map(|parent| (parent, node.node_id.clone())))
        .collect();
    for (parent, child) in links {
        if let Some(index) = by_id.get(&parent) {
            out[*index].children.push(child);
        }
    }

    Normalized {
        nodes: out,
        by_id,
        warnings,
        degraded,
    }
}
