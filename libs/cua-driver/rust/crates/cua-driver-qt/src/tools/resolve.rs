use crate::adapter::{observe, resolve_window, ObserveError, ObserveOptions};
use crate::identity::fingerprint;
use crate::model::{Node, Observation};
use crate::selector::{self, Selector};
use async_trait::async_trait;
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::{json, Value};

/// The design's decision rule: a unique top1 with a clear margin wins; a tie
/// or a thin margin is `ambiguous_target`; nothing is `target_not_found`.
const MIN_SCORE: f64 = 0.60;
const MIN_MARGIN: f64 = 0.20;
const MAX_ALTERNATIVES: usize = 5;

pub struct QtResolveTool;

static RESOLVE_DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

#[async_trait]
impl Tool for QtResolveTool {
    fn def(&self) -> &ToolDef {
        RESOLVE_DEF.get_or_init(|| ToolDef {
            name: "qt_resolve".into(),
            description: "Resolve a selector to exactly one element in a window's UIA tree and \
                return a ResolvedTarget carrying a snapshot id, element token, revision and \
                stable fingerprint. Refuses ambiguous or unprovable matches instead of \
                guessing.\n\n\
                Args: selector (e.g. `uia(name=\"OK\", controlType=\"Button\")`), plus pid \
                or window_id. M1 implements the `uia(...)` dialect only."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "Selector in the qt-test-agent dialect, e.g. uia(name=\"OK\")"
                    },
                    "pid": {
                        "type": "integer",
                        "description": "Target process id (selects its largest on-screen window)"
                    },
                    "window_id": {
                        "type": "integer",
                        "description": "Target top-level window handle"
                    },
                    "max_elements": {
                        "type": "integer",
                        "description": "Cap on walked elements (default 5000)"
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Cap on tree depth (default 25)"
                    }
                },
                "required": ["selector"],
                "additionalProperties": false
            }),
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
        })
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        let Some(selector_text) = args.get("selector").and_then(Value::as_str) else {
            return refusal("invalid_arguments", "`selector` is required".to_owned());
        };
        let selector = match selector::parse(selector_text) {
            Ok(selector) => selector,
            Err(error) => {
                let code = if error.unsupported {
                    "unsupported_selector"
                } else {
                    "invalid_selector"
                };
                return refusal(code, error.message);
            }
        };

        let pid = args.get("pid").and_then(Value::as_u64).map(|pid| pid as u32);
        let window_id = args.get("window_id").and_then(Value::as_u64);
        if pid.is_none() && window_id.is_none() {
            return refusal(
                "invalid_arguments",
                "one of `pid` or `window_id` is required".to_owned(),
            );
        }

        let options = ObserveOptions {
            max_elements: args
                .get("max_elements")
                .and_then(Value::as_u64)
                .map_or(ObserveOptions::default().max_elements, |n| n as usize),
            max_depth: args
                .get("max_depth")
                .and_then(Value::as_u64)
                .map_or(ObserveOptions::default().max_depth, |n| n as usize),
        };

        let observed = tokio::task::spawn_blocking(move || {
            let window = resolve_window(pid, window_id)?;
            let observation = observe(&window, options)?;
            Ok::<Observation, ObserveError>(observation)
        })
        .await
        .unwrap_or_else(|error| {
            Err(ObserveError {
                code: "observation_failed",
                message: format!("observation task failed: {error}"),
            })
        });

        match observed {
            Ok(observation) => render(&observation, &selector),
            Err(error) => refusal(error.code, error.message),
        }
    }
}

#[derive(Debug)]
enum Decision<'a> {
    Resolved(&'a Node, f64),
    Ambiguous(Vec<(&'a Node, f64)>),
    NotFound,
    /// The walk was truncated, so a miss cannot be read as "absent".
    Unknown,
}

fn decide<'a>(observation: &'a Observation, selector: &Selector) -> Decision<'a> {
    let mut candidates: Vec<(&Node, f64)> = observation
        .nodes
        .iter()
        .filter_map(|node| {
            let matched = selector.matches(node)?;
            (matched.score >= MIN_SCORE).then_some((node, matched.score))
        })
        .collect();

    if candidates.is_empty() {
        return if observation.elements_complete {
            Decision::NotFound
        } else {
            Decision::Unknown
        };
    }

    candidates.sort_by(|(a, a_score), (b, b_score)| {
        b_score
            .partial_cmp(a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node_id.cmp(&b.node_id))
    });

    let top_score = candidates[0].1;
    if candidates.len() == 1 {
        return Decision::Resolved(candidates[0].0, top_score);
    }
    let margin = top_score - candidates[1].1;
    if margin < MIN_MARGIN {
        Decision::Ambiguous(candidates)
    } else {
        Decision::Resolved(candidates[0].0, top_score)
    }
}

fn render(observation: &Observation, selector: &Selector) -> ToolResult {
    let matched_by = |node: &Node| -> Vec<&'static str> {
        selector
            .matches(node)
            .map(|matched| matched.matched_by)
            .unwrap_or_default()
    };

    match decide(observation, selector) {
        Decision::Resolved(node, score) => {
            let structured = json!({
                "status": "resolved",
                "matchedBy": matched_by(node),
                "score": score,
                "fingerprint": fingerprint(node),
                "rect": rect_json(node),
                "nodeId": node.node_id,
                "source": node.source.as_str(),
                "snapshotId": observation.snapshot_id,
                "elementToken": node.element_token,
                "revision": observation.revision,
                "window": window_json(observation),
                "elementsComplete": observation.elements_complete,
                "alternatives": [],
                "warnings": observation.warnings,
                "degraded": observation.degraded,
            });
            ToolResult::text(format!(
                "resolved {} {} [{}{} rev={}]",
                node.node_id,
                node.summary(),
                observation.snapshot_id,
                node.element_token
                    .as_deref()
                    .map(|token| format!(" token {token}"))
                    .unwrap_or_default(),
                observation.revision,
            ))
            .with_structured(structured)
        }
        Decision::Unknown => {
            let message = format!(
                "target_not_found cannot be proven: the UIA walk was incomplete \
                 ({} node(s), degraded: {}); re-observe before treating the target as absent",
                observation.nodes.len(),
                observation.degraded.as_deref().unwrap_or("none"),
            );
            ToolResult::error(message.clone()).with_structured(json!({
                "status": "unknown",
                "reason": "observation_incomplete",
                "message": message,
                "snapshotId": observation.snapshot_id,
                "revision": observation.revision,
                "window": window_json(observation),
                "warnings": observation.warnings,
            }))
        }
        Decision::Ambiguous(candidates) => {
            let alternatives: Vec<Value> = candidates
                .iter()
                .take(MAX_ALTERNATIVES)
                .map(|(node, score)| {
                    json!({
                        "nodeId": node.node_id,
                        "score": score,
                        "summary": node.summary(),
                    })
                })
                .collect();
            let message = format!(
                "ambiguous_target: {} candidates survive {:?}; candidates: {}",
                candidates.len(),
                selector,
                alternatives
                    .iter()
                    .map(|candidate| candidate["summary"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
            ToolResult::error(message.clone()).with_structured(json!({
                "status": "refused",
                "refusal": { "code": "ambiguous_target", "message": message },
                "snapshotId": observation.snapshot_id,
                "revision": observation.revision,
                "window": window_json(observation),
                "alternatives": alternatives,
            }))
        }
        Decision::NotFound => {
            let message = format!(
                "target_not_found: no node matched {:?} in {} node(s) \
                 (elementsComplete={})",
                selector,
                observation.nodes.len(),
                observation.elements_complete,
            );
            ToolResult::error(message.clone()).with_structured(json!({
                "status": "refused",
                "refusal": { "code": "target_not_found", "message": message },
                "snapshotId": observation.snapshot_id,
                "revision": observation.revision,
                "window": window_json(observation),
                "elementsComplete": observation.elements_complete,
                "warnings": observation.warnings,
            }))
        }
    }
}

fn rect_json(node: &Node) -> Value {
    json!({
        "x": node.rect.x,
        "y": node.rect.y,
        "w": node.rect.w,
        "h": node.rect.h,
    })
}

fn window_json(observation: &Observation) -> Value {
    json!({
        "windowId": observation.window.window_id,
        "pid": observation.window.pid,
        "title": observation.window.title,
        "x": observation.window.x,
        "y": observation.window.y,
        "width": observation.window.width,
        "height": observation.window.height,
        "scale": observation.window.scale,
    })
}

fn refusal(code: &str, message: String) -> ToolResult {
    ToolResult::error(message.clone()).with_structured(json!({
        "status": "refused",
        "refusal": { "code": code, "message": message },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Rect, Source, WindowRef};
    use std::collections::BTreeMap;
    use std::collections::HashMap;

    fn node(id: &str, name: &str) -> Node {
        Node {
            node_id: id.to_owned(),
            element_token: Some(format!("s00000001:{}", &id[1..])),
            parent_id: None,
            role: "Button".to_owned(),
            name: Some(name.to_owned()),
            automation_id: None,
            class_name: None,
            object_name: None,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            visible: true,
            enabled: true,
            states: BTreeMap::new(),
            actions: Vec::new(),
            source: Source::Uia,
            attrs: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    fn observation(nodes: Vec<Node>, elements_complete: bool) -> Observation {
        let by_id = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.node_id.clone(), index))
            .collect::<HashMap<_, _>>();
        Observation {
            snapshot_id: "s00000001".to_owned(),
            window: WindowRef {
                window_id: 1,
                pid: 1,
                title: "test".to_owned(),
                x: 0,
                y: 0,
                width: 100,
                height: 100,
                scale: 1.0,
            },
            revision: 1,
            elements_complete,
            nodes,
            by_id,
            warnings: Vec::new(),
            degraded: None,
            source_counts: BTreeMap::new(),
        }
    }

    fn selector(text: &str) -> Selector {
        selector::parse(text).unwrap()
    }

    #[test]
    fn duplicate_matches_are_ambiguous_not_first_wins() {
        let observation = observation(vec![node("n1", "OK"), node("n2", "OK")], true);
        assert!(matches!(
            decide(&observation, &selector(r#"uia(name="OK")"#)),
            Decision::Ambiguous(candidates) if candidates.len() == 2
        ));
    }

    #[test]
    fn unique_match_resolves_with_its_token() {
        let observation = observation(vec![node("n1", "OK"), node("n2", "Cancel")], true);
        let Decision::Resolved(node, score) = decide(&observation, &selector(r#"uia(name="OK")"#))
        else {
            panic!("expected a resolution");
        };
        assert_eq!(node.node_id, "n1");
        assert_eq!(score, 1.0);
    }

    #[test]
    fn missing_target_on_a_complete_walk_is_not_found() {
        let observation = observation(vec![node("n1", "OK")], true);
        assert!(matches!(
            decide(&observation, &selector(r#"uia(name="Cancel")"#)),
            Decision::NotFound
        ));
    }

    #[test]
    fn missing_target_on_an_incomplete_walk_is_unknown() {
        let observation = observation(vec![node("n1", "OK")], false);
        assert!(matches!(
            decide(&observation, &selector(r#"uia(name="Cancel")"#)),
            Decision::Unknown
        ));
    }
}
