//! `graph.json`: Obsidian's graph view settings. Unknown keys are preserved by keeping the JSON
//! document and reading/writing only the keys that are understood.

use crate::layout::Forces;
use crate::model::Filters;
use serde_json::{json, Value};

#[derive(Clone, PartialEq, Debug)]
pub struct ColorGroup {
    pub query: String,
    /// 0xRRGGBB
    pub rgb: u32,
    pub alpha: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct GraphSettings {
    pub search: String,
    pub filters: Filters,
    pub color_groups: Vec<ColorGroup>,
    pub forces: Forces,
    pub show_arrows: bool,
    pub text_fade: f32,
    pub node_size: f32,
    pub line_size: f32,
    pub scale: f32,
}

impl Default for GraphSettings {
    fn default() -> Self {
        GraphSettings {
            search: String::new(),
            filters: Filters::default(),
            color_groups: Vec::new(),
            forces: Forces::default(),
            show_arrows: false,
            text_fade: 0.0,
            node_size: 1.0,
            line_size: 1.0,
            scale: 1.0,
        }
    }
}

fn f(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key).and_then(Value::as_f64).map_or(default, |x| x as f32)
}
fn b(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

impl GraphSettings {
    pub fn from_json(v: &Value) -> GraphSettings {
        let d = GraphSettings::default();
        let color_groups = v
            .get("colorGroups")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(|g| {
                        let color = g.get("color")?;
                        Some(ColorGroup {
                            query: g.get("query")?.as_str()?.to_string(),
                            rgb: color.get("rgb")?.as_u64()? as u32 & 0x00ff_ffff,
                            alpha: f(color, "a", 1.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        GraphSettings {
            search: v.get("search").and_then(Value::as_str).unwrap_or("").to_string(),
            filters: Filters {
                show_tags: b(v, "showTags", d.filters.show_tags),
                show_attachments: b(v, "showAttachments", d.filters.show_attachments),
                hide_unresolved: b(v, "hideUnresolved", d.filters.hide_unresolved),
                show_orphans: b(v, "showOrphans", d.filters.show_orphans),
            },
            color_groups,
            forces: Forces {
                center_strength: f(v, "centerStrength", d.forces.center_strength),
                repel_strength: f(v, "repelStrength", d.forces.repel_strength),
                link_strength: f(v, "linkStrength", d.forces.link_strength),
                link_distance: f(v, "linkDistance", d.forces.link_distance),
            },
            show_arrows: b(v, "showArrow", d.show_arrows),
            text_fade: f(v, "textFadeMultiplier", d.text_fade),
            node_size: f(v, "nodeSizeMultiplier", d.node_size),
            line_size: f(v, "lineSizeMultiplier", d.line_size),
            scale: f(v, "scale", d.scale),
        }
    }

    /// Write the known keys into `v`, leaving every other key untouched.
    pub fn write_into(&self, v: &mut Value) {
        if !v.is_object() {
            *v = json!({});
        }
        let Some(o) = v.as_object_mut() else { return };
        let num = |x: f32| json!((x as f64 * 1e6).round() / 1e6);
        o.insert("search".into(), json!(self.search));
        o.insert("showTags".into(), json!(self.filters.show_tags));
        o.insert("showAttachments".into(), json!(self.filters.show_attachments));
        o.insert("hideUnresolved".into(), json!(self.filters.hide_unresolved));
        o.insert("showOrphans".into(), json!(self.filters.show_orphans));
        let groups: Vec<Value> = self
            .color_groups
            .iter()
            .map(|g| json!({"query": g.query, "color": {"a": num(g.alpha), "rgb": g.rgb}}))
            .collect();
        o.insert("colorGroups".into(), Value::Array(groups));
        o.insert("showArrow".into(), json!(self.show_arrows));
        o.insert("textFadeMultiplier".into(), num(self.text_fade));
        o.insert("nodeSizeMultiplier".into(), num(self.node_size));
        o.insert("lineSizeMultiplier".into(), num(self.line_size));
        o.insert("centerStrength".into(), num(self.forces.center_strength));
        o.insert("repelStrength".into(), num(self.forces.repel_strength));
        o.insert("linkStrength".into(), num(self.forces.link_strength));
        o.insert("linkDistance".into(), num(self.forces.link_distance));
        o.insert("scale".into(), num(self.scale));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBSIDIAN: &str = r##"{
  "collapse-filter": true,
  "search": "tag:#project",
  "showTags": true,
  "showAttachments": false,
  "hideUnresolved": true,
  "showOrphans": false,
  "collapse-color-groups": false,
  "colorGroups": [
    { "query": "path:Daily", "color": { "a": 1, "rgb": 14701138 } }
  ],
  "collapse-display": true,
  "showArrow": true,
  "textFadeMultiplier": -1.2,
  "nodeSizeMultiplier": 1.5,
  "lineSizeMultiplier": 1,
  "collapse-forces": true,
  "centerStrength": 0.4,
  "repelStrength": 12,
  "linkStrength": 0.8,
  "linkDistance": 200,
  "scale": 0.75,
  "close": false
}"##;

    #[test]
    fn reads_obsidian_graph_json_and_preserves_unknown_keys() {
        let mut v: Value = serde_json::from_str(OBSIDIAN).unwrap();
        let mut s = GraphSettings::from_json(&v);
        assert_eq!(s.search, "tag:#project");
        assert!(s.filters.show_tags && s.filters.hide_unresolved && !s.filters.show_orphans);
        assert_eq!(
            s.color_groups,
            vec![ColorGroup { query: "path:Daily".into(), rgb: 0xe05252, alpha: 1.0 }]
        );
        assert_eq!((s.forces.repel_strength, s.forces.link_distance, s.scale), (12.0, 200.0, 0.75));
        s.forces.repel_strength = 15.0;
        s.write_into(&mut v);
        assert_eq!(v["collapse-filter"], json!(true));
        assert_eq!(v["close"], json!(false));
        assert_eq!(v["repelStrength"], json!(15.0));
        assert_eq!(GraphSettings::from_json(&v), s);
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys[0], "collapse-filter", "key order is kept");
    }

    #[test]
    fn defaults_for_missing_file() {
        assert_eq!(GraphSettings::from_json(&json!({})), GraphSettings::default());
    }
}
