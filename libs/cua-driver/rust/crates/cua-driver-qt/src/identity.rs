use crate::model::Node;
use sha2::{Digest, Sha256};

/// Identity is the element's stable shape, not its snapshot handle: the same
/// control observed again produces the same fingerprint.
pub fn fingerprint(node: &Node) -> String {
    let canonical = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{:.0},{:.0},{:.0},{:.0}",
        node.role,
        node.name.as_deref().unwrap_or(""),
        node.automation_id.as_deref().unwrap_or(""),
        node.object_name.as_deref().unwrap_or(""),
        node.rect.x,
        node.rect.y,
        node.rect.w,
        node.rect.h,
    );
    let digest = Sha256::digest(canonical.as_bytes());
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Rect, Source};
    use std::collections::BTreeMap;

    fn node(name: &str) -> Node {
        Node {
            node_id: "n1".to_owned(),
            element_token: None,
            parent_id: None,
            role: "Button".to_owned(),
            name: Some(name.to_owned()),
            automation_id: None,
            class_name: None,
            object_name: None,
            rect: Rect {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 40.0,
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
    fn fingerprint_ignores_the_observation_handle() {
        let mut first = node("OK");
        first.node_id = "n7".to_owned();
        let mut second = node("OK");
        second.node_id = "n42".to_owned();
        assert_eq!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn fingerprint_changes_with_identity_fields() {
        assert_ne!(fingerprint(&node("OK")), fingerprint(&node("Cancel")));
    }
}
