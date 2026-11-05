use crate::model::Node;

#[derive(Clone, Debug, PartialEq)]
pub struct UiaSelector {
    pub name: Option<String>,
    pub control_type: Option<String>,
    pub automation_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Selector {
    Uia(UiaSelector),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub matched_by: Vec<&'static str>,
    pub score: f64,
}

#[derive(Debug, PartialEq)]
pub struct SelectorError {
    pub dialect: &'static str,
    /// True when the dialect itself is not implemented yet; false for a
    /// malformed selector in a supported dialect.
    pub unsupported: bool,
    pub message: String,
}

impl SelectorError {
    fn invalid(dialect: &'static str, message: String) -> Self {
        Self {
            dialect,
            unsupported: false,
            message,
        }
    }
}

/// Parse the selector dialect. M1 only implements `uia(...)`; other documented
/// dialects (`widget`, `path`, `region`) are recognized so callers get a
/// precise "not implemented yet" instead of a parse failure.
pub fn parse(input: &str) -> Result<Selector, SelectorError> {
    let input = input.trim();
    let (dialect, rest) = match input.split_once('(') {
        Some(parts) => parts,
        None => {
            return Err(SelectorError::invalid(
                "unknown",
                format!("selector must be a call: {input:?}"),
            ))
        }
    };
    if !rest.ends_with(')') {
        return Err(SelectorError::invalid(
            "unknown",
            format!("selector is missing its closing ')': {input:?}"),
        ));
    }
    let body = &rest[..rest.len() - 1];
    match dialect.trim() {
        "uia" => Ok(Selector::Uia(parse_uia(body)?)),
        other @ ("widget" | "path" | "region" | "qml") => Err(SelectorError {
            dialect: match other {
                "widget" => "widget",
                "path" => "path",
                "region" => "region",
                _ => "qml",
            },
            unsupported: true,
            message: format!("selector dialect {other:?} is not implemented yet"),
        }),
        other => Err(SelectorError::invalid(
            "unknown",
            format!("unknown selector dialect {other:?}"),
        )),
    }
}

fn parse_uia(body: &str) -> Result<UiaSelector, SelectorError> {
    let mut selector = UiaSelector {
        name: None,
        control_type: None,
        automation_id: None,
    };
    for arg in split_args(body)? {
        let Some((key, value)) = arg.split_once('=') else {
            return Err(SelectorError::invalid(
                "uia",
                format!("expected key=value in {arg:?}"),
            ));
        };
        let key = key.trim();
        let value = unquote(value.trim())?;
        match key {
            "name" | "text" => selector.name = Some(value),
            "controlType" => selector.control_type = Some(value),
            "automationId" => selector.automation_id = Some(value),
            other => {
                return Err(SelectorError::invalid(
                    "uia",
                    format!("unsupported uia() argument {other:?}"),
                ))
            }
        }
    }
    if selector.name.is_none()
        && selector.control_type.is_none()
        && selector.automation_id.is_none()
    {
        return Err(SelectorError::invalid(
            "uia",
            "uia() needs at least one criterion".to_owned(),
        ));
    }
    Ok(selector)
}

fn split_args(body: &str) -> Result<Vec<String>, SelectorError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in body.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some(_), '\\') => {
                current.push(ch);
                escaped = true;
            }
            (Some(q), c) if c == q => {
                quote = None;
                current.push(c);
            }
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(ch);
                current.push(ch);
            }
            (None, ',') => {
                if !current.trim().is_empty() {
                    args.push(current.trim().to_owned());
                }
                current.clear();
            }
            (None, c) => current.push(c),
        }
    }
    if quote.is_some() {
        return Err(SelectorError::invalid(
            "uia",
            "unterminated quoted value".to_owned(),
        ));
    }
    if !current.trim().is_empty() {
        args.push(current.trim().to_owned());
    }
    Ok(args)
}

fn unquote(value: &str) -> Result<String, SelectorError> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 {
        return Err(SelectorError::invalid(
            "uia",
            format!("value {value:?} must be quoted"),
        ));
    }
    let quote = bytes[0];
    if (quote != b'"' && quote != b'\'') || bytes[bytes.len() - 1] != quote {
        return Err(SelectorError::invalid(
            "uia",
            format!("value {value:?} must be quoted"),
        ));
    }
    let inner = &value[1..value.len() - 1];
    Ok(inner.replace("\\\"", "\"").replace("\\'", "'"))
}

impl Selector {
    pub fn matches(&self, node: &Node) -> Option<Match> {
        match self {
            Selector::Uia(selector) => selector.matches(node),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Selector::Uia(selector) => {
                selector.name.is_none()
                    && selector.control_type.is_none()
                    && selector.automation_id.is_none()
            }
        }
    }
}

impl UiaSelector {
    fn matches(&self, node: &Node) -> Option<Match> {
        let mut matched_by = Vec::new();

        if let Some(automation_id) = &self.automation_id {
            if node.automation_id.as_deref() != Some(automation_id.as_str()) {
                return None;
            }
            matched_by.push("automationId");
        }

        if let Some(control_type) = &self.control_type {
            if !node.role.eq_ignore_ascii_case(control_type) {
                return None;
            }
            matched_by.push("controlType");
        }

        if let Some(name) = &self.name {
            if node.name.as_deref() != Some(name.as_str()) {
                return None;
            }
            matched_by.push("name");
        }

        // All criteria are hard filters, so a surviving candidate matches every
        // provided criterion. Score stays 1.0; ambiguity is decided by the
        // candidate count and the top1/top2 margin in the resolver.
        Some(Match {
            matched_by,
            score: 1.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Rect, Source};
    use std::collections::BTreeMap;

    fn node(role: &str, name: Option<&str>, automation_id: Option<&str>) -> Node {
        Node {
            node_id: "n1".to_owned(),
            element_token: None,
            parent_id: None,
            role: role.to_owned(),
            name: name.map(str::to_owned),
            automation_id: automation_id.map(str::to_owned),
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

    #[test]
    fn parses_all_three_criteria() {
        let selector = parse(r#"uia(name="开始扫描", controlType="Button", automationId="scanStart")"#)
            .unwrap();
        assert_eq!(
            selector,
            Selector::Uia(UiaSelector {
                name: Some("开始扫描".to_owned()),
                control_type: Some("Button".to_owned()),
                automation_id: Some("scanStart".to_owned()),
            })
        );
    }

    #[test]
    fn quoted_comma_stays_inside_the_value() {
        let selector = parse(r#"uia(name="A, B")"#).unwrap();
        assert_eq!(
            selector,
            Selector::Uia(UiaSelector {
                name: Some("A, B".to_owned()),
                control_type: None,
                automation_id: None,
            })
        );
    }

    #[test]
    fn rejects_unknown_dialect_and_argument() {
        assert_eq!(parse("widget(objectName=\"x\")").unwrap_err().dialect, "widget");
        assert_eq!(
            parse("uia(className=\"QPushButton\")").unwrap_err().dialect,
            "uia"
        );
        assert!(parse("uia()").is_err());
        assert!(parse("uia(name=x)").is_err());
    }

    #[test]
    fn distinguishes_unimplemented_dialects_from_malformed_selectors() {
        assert!(parse("widget(objectName=\"x\")").unwrap_err().unsupported);
        assert!(!parse("uia(name=x)").unwrap_err().unsupported);
        assert!(!parse("uia()").unwrap_err().unsupported);
    }

    #[test]
    fn matching_requires_every_criterion() {
        let selector =
            parse(r#"uia(name="启动", controlType="Button")"#).unwrap();
        let hit = node("Button", Some("启动"), None);
        assert_eq!(
            selector.matches(&hit).unwrap().matched_by,
            vec!["controlType", "name"]
        );
        assert!(selector.matches(&node("Edit", Some("启动"), None)).is_none());
        assert!(selector.matches(&node("Button", Some("停止"), None)).is_none());
    }
}
