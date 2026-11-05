use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    QtBridge,
    Uia,
    Pixel,
    Dom,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::QtBridge => "qt_bridge",
            Source::Uia => "uia",
            Source::Pixel => "pixel",
            Source::Dom => "dom",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub node_id: String,
    pub element_token: Option<String>,
    pub parent_id: Option<String>,
    pub role: String,
    pub name: Option<String>,
    pub automation_id: Option<String>,
    pub class_name: Option<String>,
    pub object_name: Option<String>,
    pub rect: Rect,
    pub visible: bool,
    pub enabled: bool,
    pub states: BTreeMap<String, String>,
    pub actions: Vec<String>,
    pub source: Source,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<String>,
}

impl Node {
    pub fn summary(&self) -> String {
        let mut out = format!("{}", self.role);
        if let Some(name) = &self.name {
            out.push_str(&format!(" \"{name}\""));
        }
        if let Some(id) = &self.automation_id {
            out.push_str(&format!(" #{id}"));
        }
        out.push_str(&format!(
            " @ ({:.0},{:.0}) {:.0}x{:.0}",
            self.rect.x, self.rect.y, self.rect.w, self.rect.h
        ));
        out
    }
}

#[derive(Clone, Debug)]
pub struct WindowRef {
    pub window_id: u64,
    pub pid: u32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
}

#[derive(Debug)]
pub struct Observation {
    pub snapshot_id: String,
    pub window: WindowRef,
    pub revision: u64,
    pub elements_complete: bool,
    pub nodes: Vec<Node>,
    pub by_id: HashMap<String, usize>,
    pub warnings: Vec<String>,
    pub degraded: Option<String>,
    pub source_counts: BTreeMap<Source, usize>,
}

impl Observation {
    pub fn node(&self, node_id: &str) -> Option<&Node> {
        self.by_id.get(node_id).map(|index| &self.nodes[*index])
    }
}
