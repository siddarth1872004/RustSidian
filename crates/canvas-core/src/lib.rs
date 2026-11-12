//! JSON Canvas 1.0 (<https://jsoncanvas.org>): the open format behind `.canvas` files.
//!
//! Every node and edge keeps its original JSON object, so fields this crate does not know are
//! written back unchanged and in their original order.

use serde_json::{json, Map, Value};

#[derive(Clone, PartialEq, Debug)]
pub enum NodeKind {
    Text {
        text: String,
    },
    File {
        file: String,
        subpath: Option<String>,
    },
    Link {
        url: String,
    },
    Group {
        label: Option<String>,
    },
    /// A node type this crate does not know; kept as is.
    Other(String),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// A preset `"1"`..`"6"` or a hex color.
    pub color: Option<String>,
    raw: Map<String, Value>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    fn parse(s: &str) -> Option<Side> {
        Some(match s {
            "top" => Side::Top,
            "right" => Side::Right,
            "bottom" => Side::Bottom,
            "left" => Side::Left,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Side::Top => "top",
            Side::Right => "right",
            Side::Bottom => "bottom",
            Side::Left => "left",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Edge {
    pub id: String,
    pub from_node: String,
    pub from_side: Option<Side>,
    pub to_node: String,
    pub to_side: Option<Side>,
    /// `fromEnd` defaults to none, `toEnd` to arrow.
    pub from_arrow: bool,
    pub to_arrow: bool,
    pub color: Option<String>,
    pub label: Option<String>,
    raw: Map<String, Value>,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Canvas {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Top-level keys other than `nodes` and `edges`.
    extra: Map<String, Value>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid canvas: {}", self.0)
    }
}
impl std::error::Error for Error {}

fn str_of(o: &Map<String, Value>, k: &str) -> Option<String> {
    o.get(k).and_then(Value::as_str).map(str::to_string)
}
fn num(o: &Map<String, Value>, k: &str) -> f64 {
    o.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

impl Node {
    fn from_json(v: &Value) -> Option<Node> {
        let o = v.as_object()?;
        let kind = match o.get("type").and_then(Value::as_str)? {
            "text" => NodeKind::Text { text: str_of(o, "text").unwrap_or_default() },
            "file" => {
                NodeKind::File { file: str_of(o, "file").unwrap_or_default(), subpath: str_of(o, "subpath") }
            }
            "link" => NodeKind::Link { url: str_of(o, "url").unwrap_or_default() },
            "group" => NodeKind::Group { label: str_of(o, "label") },
            other => NodeKind::Other(other.to_string()),
        };
        Some(Node {
            id: str_of(o, "id")?,
            kind,
            x: num(o, "x"),
            y: num(o, "y"),
            width: num(o, "width"),
            height: num(o, "height"),
            color: str_of(o, "color"),
            raw: o.clone(),
        })
    }

    fn to_json(&self) -> Value {
        let mut o = self.raw.clone();
        let int = |v: f64| json!(v.round() as i64);
        o.insert("id".into(), json!(self.id));
        let set_opt = |o: &mut Map<String, Value>, k: &str, v: &Option<String>| match v {
            Some(v) => {
                o.insert(k.into(), json!(v));
            }
            None => {
                o.shift_remove(k);
            }
        };
        match &self.kind {
            NodeKind::Text { text } => {
                o.insert("type".into(), json!("text"));
                o.insert("text".into(), json!(text));
            }
            NodeKind::File { file, subpath } => {
                o.insert("type".into(), json!("file"));
                o.insert("file".into(), json!(file));
                set_opt(&mut o, "subpath", subpath);
            }
            NodeKind::Link { url } => {
                o.insert("type".into(), json!("link"));
                o.insert("url".into(), json!(url));
            }
            NodeKind::Group { label } => {
                o.insert("type".into(), json!("group"));
                set_opt(&mut o, "label", label);
            }
            NodeKind::Other(t) => {
                o.insert("type".into(), json!(t));
            }
        }
        o.insert("x".into(), int(self.x));
        o.insert("y".into(), int(self.y));
        o.insert("width".into(), int(self.width));
        o.insert("height".into(), int(self.height));
        set_opt(&mut o, "color", &self.color);
        Value::Object(o)
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.width && y <= self.y + self.height
    }

    /// Where an edge attaches on the given side.
    pub fn anchor(&self, side: Side) -> (f64, f64) {
        match side {
            Side::Top => (self.x + self.width / 2.0, self.y),
            Side::Bottom => (self.x + self.width / 2.0, self.y + self.height),
            Side::Left => (self.x, self.y + self.height / 2.0),
            Side::Right => (self.x + self.width, self.y + self.height / 2.0),
        }
    }
}

impl Edge {
    fn from_json(v: &Value) -> Option<Edge> {
        let o = v.as_object()?;
        let side = |k: &str| o.get(k).and_then(Value::as_str).and_then(Side::parse);
        let end = |k: &str, default: bool| o.get(k).and_then(Value::as_str).map_or(default, |v| v == "arrow");
        Some(Edge {
            id: str_of(o, "id")?,
            from_node: str_of(o, "fromNode")?,
            from_side: side("fromSide"),
            to_node: str_of(o, "toNode")?,
            to_side: side("toSide"),
            from_arrow: end("fromEnd", false),
            to_arrow: end("toEnd", true),
            color: str_of(o, "color"),
            label: str_of(o, "label"),
            raw: o.clone(),
        })
    }

    fn to_json(&self) -> Value {
        let mut o = self.raw.clone();
        o.insert("id".into(), json!(self.id));
        o.insert("fromNode".into(), json!(self.from_node));
        o.insert("toNode".into(), json!(self.to_node));
        for (k, v) in [("fromSide", self.from_side), ("toSide", self.to_side)] {
            match v {
                Some(s) => {
                    o.insert(k.into(), json!(s.name()));
                }
                None => {
                    o.shift_remove(k);
                }
            }
        }
        // Only write the end markers when they differ from the defaults or were present.
        if self.from_arrow || o.contains_key("fromEnd") {
            o.insert("fromEnd".into(), json!(if self.from_arrow { "arrow" } else { "none" }));
        }
        if !self.to_arrow || o.contains_key("toEnd") {
            o.insert("toEnd".into(), json!(if self.to_arrow { "arrow" } else { "none" }));
        }
        for (k, v) in [("color", &self.color), ("label", &self.label)] {
            match v {
                Some(s) => {
                    o.insert(k.into(), json!(s));
                }
                None => {
                    o.shift_remove(k);
                }
            }
        }
        Value::Object(o)
    }
}

impl Canvas {
    /// Parse a `.canvas` file. An empty file is an empty canvas (Obsidian creates those).
    pub fn parse(src: &str) -> Result<Canvas, Error> {
        if src.trim().is_empty() {
            return Ok(Canvas::default());
        }
        let v: Value = serde_json::from_str(src).map_err(|e| Error(e.to_string()))?;
        let Value::Object(mut top) = v else { return Err(Error("top level must be an object".into())) };
        let list =
            |v: Option<Value>| -> Vec<Value> { v.and_then(|v| v.as_array().cloned()).unwrap_or_default() };
        let nodes: Vec<Node> = list(top.shift_remove("nodes")).iter().filter_map(Node::from_json).collect();
        let edges: Vec<Edge> = list(top.shift_remove("edges")).iter().filter_map(Edge::from_json).collect();
        Ok(Canvas { nodes, edges, extra: top })
    }

    /// Serialize in Obsidian's layout: tab indentation at the top, one node or edge per line.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        let compact = |v: &Value| serde_json::to_string(v).unwrap_or_default();
        let section = |name: &str, items: Vec<Value>| {
            let body: Vec<String> = items.iter().map(|v| format!("\t\t{}", compact(v))).collect();
            if body.is_empty() {
                format!("\t\"{name}\":[]")
            } else {
                format!("\t\"{name}\":[\n{}\n\t]", body.join(",\n"))
            }
        };
        let mut parts = vec![
            section("nodes", self.nodes.iter().map(Node::to_json).collect()),
            section("edges", self.edges.iter().map(Edge::to_json).collect()),
        ];
        for (k, v) in &self.extra {
            parts.push(format!("\t{}:{}", compact(&json!(k)), compact(v)));
        }
        out.push_str(&parts.join(",\n"));
        out.push_str("\n}");
        out
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// A 16-hex-digit id not used in this canvas, derived from `seed`.
    pub fn new_id(&self, seed: u64) -> String {
        let mut s = seed ^ 0x9e37_79b9_7f4a_7c15;
        loop {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let id = format!("{s:016x}");
            if self.node(&id).is_none() && !self.edges.iter().any(|e| e.id == id) {
                return id;
            }
        }
    }

    pub fn add_node(&mut self, kind: NodeKind, x: f64, y: f64, width: f64, height: f64, seed: u64) -> String {
        let id = self.new_id(seed);
        self.nodes.push(Node { id: id.clone(), kind, x, y, width, height, color: None, raw: Map::new() });
        id
    }

    /// Connect two nodes. Returns `None` if either does not exist.
    pub fn add_edge(&mut self, from: &str, to: &str, seed: u64) -> Option<String> {
        self.node(from)?;
        self.node(to)?;
        let id = self.new_id(seed);
        self.edges.push(Edge {
            id: id.clone(),
            from_node: from.into(),
            from_side: None,
            to_node: to.into(),
            to_side: None,
            from_arrow: false,
            to_arrow: true,
            color: None,
            label: None,
            raw: Map::new(),
        });
        Some(id)
    }

    /// Remove a node and every edge attached to it.
    pub fn remove_node(&mut self, id: &str) -> bool {
        let before = self.nodes.len();
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from_node != id && e.to_node != id);
        self.nodes.len() != before
    }

    /// Move a node. Moving a group moves the nodes fully inside it.
    pub fn move_node(&mut self, id: &str, dx: f64, dy: f64) {
        let Some(n) = self.node(id) else { return };
        let mut ids = vec![id.to_string()];
        if matches!(n.kind, NodeKind::Group { .. }) {
            ids.extend(self.members(id));
        }
        for n in self.nodes.iter_mut().filter(|n| ids.contains(&n.id)) {
            n.x += dx;
            n.y += dy;
        }
    }

    /// Ids of the nodes that lie fully inside a group.
    pub fn members(&self, group: &str) -> Vec<String> {
        let Some(g) = self.node(group) else { return Vec::new() };
        self.nodes
            .iter()
            .filter(|n| {
                n.id != group
                    && n.x >= g.x
                    && n.y >= g.y
                    && n.x + n.width <= g.x + g.width
                    && n.y + n.height <= g.y + g.height
            })
            .map(|n| n.id.clone())
            .collect()
    }

    /// Topmost node at a point. Later nodes are drawn above earlier ones; groups are below.
    pub fn hit(&self, x: f64, y: f64) -> Option<&Node> {
        let hits = || self.nodes.iter().rev().filter(move |n| n.contains(x, y));
        hits().find(|n| !matches!(n.kind, NodeKind::Group { .. })).or_else(|| hits().next())
    }

    /// Bounding box (x, y, width, height) of all nodes.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let first = self.nodes.first()?;
        let (mut x0, mut y0, mut x1, mut y1) =
            (first.x, first.y, first.x + first.width, first.y + first.height);
        for n in &self.nodes {
            x0 = x0.min(n.x);
            y0 = y0.min(n.y);
            x1 = x1.max(n.x + n.width);
            y1 = y1.max(n.y + n.height);
        }
        Some((x0, y0, x1 - x0, y1 - y0))
    }

    /// Vault paths referenced by file nodes (for the link graph and rename propagation).
    pub fn file_links(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::File { file, .. } => Some(file.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Point file nodes at a renamed path. Returns how many nodes changed.
    pub fn rename_file(&mut self, old: &str, new: &str) -> usize {
        let mut n = 0;
        for node in &mut self.nodes {
            if let NodeKind::File { file, .. } = &mut node.kind {
                if file == old {
                    *file = new.to_string();
                    n += 1;
                }
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
	"nodes":[
		{"id":"g1","type":"group","x":-40,"y":-40,"width":500,"height":300,"label":"Ideas","background":"bg.png","backgroundStyle":"cover"},
		{"id":"a","type":"text","text":"# Hello\n**md**","x":0,"y":0,"width":250,"height":120,"color":"4"},
		{"id":"b","type":"file","file":"Notes/Plan.md","subpath":"#Goals","x":260,"y":20,"width":180,"height":100,"futureField":{"k":[1,2]}},
		{"id":"c","type":"link","url":"https://jsoncanvas.org","x":600,"y":0,"width":300,"height":200,"color":"#ff0000"},
		{"id":"z","type":"hologram","x":0,"y":900,"width":10,"height":10}
	],
	"edges":[
		{"id":"e1","fromNode":"a","fromSide":"right","toNode":"b","toSide":"left","label":"leads to"},
		{"id":"e2","fromNode":"b","toNode":"c","fromEnd":"arrow","toEnd":"none","color":"2"}
	],
	"metadata":{"version":"1.0-1.0"}
}"##;

    #[test]
    fn parse_and_round_trip_is_lossless() {
        let c = Canvas::parse(SAMPLE).unwrap();
        assert_eq!((c.nodes.len(), c.edges.len()), (5, 2));
        assert_eq!(
            c.node("b").unwrap().kind,
            NodeKind::File { file: "Notes/Plan.md".into(), subpath: Some("#Goals".into()) }
        );
        assert_eq!(c.node("z").unwrap().kind, NodeKind::Other("hologram".into()));
        let e2 = &c.edges[1];
        assert!(e2.from_arrow && !e2.to_arrow && c.edges[0].to_arrow && !c.edges[0].from_arrow);
        assert_eq!(c.edges[0].from_side, Some(Side::Right));
        assert_eq!(c.to_json(), SAMPLE, "unknown fields, key order and layout are preserved");
        assert_eq!(Canvas::parse("").unwrap(), Canvas::default());
        assert_eq!(Canvas::parse("  ").unwrap().to_json(), "{\n\t\"nodes\":[],\n\t\"edges\":[]\n}");
        assert!(Canvas::parse("[1]").is_err() && Canvas::parse("{nope").is_err());
    }

    #[test]
    fn editing() {
        let mut c = Canvas::parse(SAMPLE).unwrap();
        assert_eq!(c.members("g1"), vec!["a", "b"]);
        c.move_node("g1", 10.0, 5.0);
        assert_eq!(
            (c.node("a").unwrap().x, c.node("b").unwrap().y, c.node("c").unwrap().x),
            (10.0, 25.0, 600.0)
        );
        assert_eq!(c.hit(20.0, 20.0).unwrap().id, "a", "cards are above the group that contains them");
        assert_eq!(c.hit(-20.0, -20.0).unwrap().id, "g1");
        assert!(c.hit(5000.0, 0.0).is_none());
        let n = c.add_node(NodeKind::Text { text: "new".into() }, 1000.0, 0.0, 100.0, 50.0, 7);
        assert_eq!(n.len(), 16);
        let e = c.add_edge("a", &n, 8).unwrap();
        assert!(c.add_edge("a", "missing", 9).is_none());
        assert!(c.remove_node("b"));
        assert_eq!(
            c.edges.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec![e.as_str()],
            "edges of a removed node go with it"
        );
        assert_eq!(c.rename_file("x", "y"), 0);
        let again = Canvas::parse(&c.to_json()).unwrap();
        assert_eq!(again, Canvas::parse(&again.to_json()).unwrap());
        assert_eq!(again.node(&n).unwrap().kind, NodeKind::Text { text: "new".into() });
        assert_eq!(c.bounds(), Some((-30.0, -35.0, 1130.0, 945.0)));
        assert_eq!(c.node("a").unwrap().anchor(Side::Right), (260.0, 65.0));
    }

    #[test]
    fn file_links_and_rename() {
        let mut c = Canvas::parse(SAMPLE).unwrap();
        assert_eq!(c.file_links(), vec!["Notes/Plan.md"]);
        assert_eq!(c.rename_file("Notes/Plan.md", "Archive/Plan.md"), 1);
        assert!(c.to_json().contains("\"file\":\"Archive/Plan.md\"") && c.to_json().contains("futureField"));
    }
}
