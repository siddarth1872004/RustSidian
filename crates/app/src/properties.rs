//! Properties editor: the YAML front matter of a note shown as an editable key/value table.
//! Every change is a text edit of the buffer, so undo and saving work as for any other edit.

use crate::shell::{App, Key, ViewMode};
use editor_core::{Change, EditKind, Sel};
use md_engine::frontmatter::{self, Value};
use md_engine::NodeKind;
use vault_core::fsio;
use walnut_ui::{Dom, El, NodeId};

pub struct Entry {
    pub key: String,
    pub value: Value,
    /// Byte range of the entry's lines (key line through its last value line).
    pub start: usize,
    pub end: usize,
}

pub struct Front {
    pub entries: Vec<Entry>,
    /// Offset of the closing fence line.
    pub close: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,
    List,
    Number,
    Checkbox,
    Date,
    Other,
}

const LIST_KEYS: [&str; 6] = ["tags", "tag", "aliases", "alias", "cssclasses", "cssclass"];

pub fn kind(key: &str, v: &Value) -> Kind {
    let listy = LIST_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key));
    match v {
        Value::List(items) if items.iter().all(|i| !matches!(i, Value::Map(_) | Value::List(_))) => Kind::List,
        Value::List(_) | Value::Map(_) => Kind::Other,
        Value::Null if listy => Kind::List,
        Value::String(_) if listy => Kind::List,
        Value::Bool(_) => Kind::Checkbox,
        Value::Number(_) => Kind::Number,
        Value::String(s) if is_date(s) => Kind::Date,
        _ => Kind::Text,
    }
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|x| x.iter().all(u8::is_ascii_digit));
    b.len() >= 10
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && (b.len() == 10 || (b[10] == b'T' && digits(11..13) && b.get(13) == Some(&b':')))
}

fn list_items(v: &Value) -> Vec<Value> {
    match v {
        Value::List(items) => items.clone(),
        other => other.as_string_list().into_iter().map(Value::String).collect(),
    }
}

/// Parse the front matter at the start of `text` (fences included). `Err` when there is none or
/// it is not valid YAML, in which case the source is shown instead.
pub fn scan(text: &str) -> Result<Front, ()> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().ok_or(())?;
    if first.trim_end() != "---" {
        return Err(());
    }
    let mut at = first.len();
    let mut body: Vec<(usize, &str)> = Vec::new();
    let mut close = None;
    for l in lines {
        if matches!(l.trim_end(), "---" | "...") {
            close = Some(at);
            break;
        }
        body.push((at, l));
        at += l.len();
    }
    let close = close.ok_or(())?;
    let is_key_line = |l: &str| {
        !l.is_empty() && !l.starts_with([' ', '\t', '#', '\n', '\r']) && !l.starts_with("- ") && l != "-\n"
    };
    let starts: Vec<usize> = (0..body.len()).filter(|&i| is_key_line(body[i].1)).collect();
    let content = &text[first.len()..close];
    frontmatter::parse(content).map_err(|_| ())?;
    let mut entries = Vec::new();
    for (n, &i) in starts.iter().enumerate() {
        let start = body[i].0;
        let mut end = starts.get(n + 1).map_or(close, |&j| body[j].0);
        // Comments and blank lines after an entry belong to the gap, not to the entry.
        while end > start {
            let prev = text[..end].trim_end_matches('\n').rfind('\n').map_or(0, |p| p + 1);
            let line = text[prev..end].trim();
            if prev > start && (line.is_empty() || line.starts_with('#')) {
                end = prev;
            } else {
                break;
            }
        }
        let (key, value) = frontmatter::parse(&text[start..end]).map_err(|_| ())?.into_iter().next().ok_or(())?;
        entries.push(Entry { key, value, start, end });
    }
    Ok(Front { entries, close })
}

fn serialize(key: &str, v: &Value) -> String {
    frontmatter::to_string(&[(key.to_string(), v.clone())])
}

impl Front {
    pub fn set_value(&self, i: usize, v: &Value) -> Change {
        let e = &self.entries[i];
        Change { start: e.start, end: e.end, text: serialize(&e.key, v) }
    }
    pub fn remove(&self, i: usize) -> Change {
        let e = &self.entries[i];
        Change { start: e.start, end: e.end, text: String::new() }
    }
    pub fn rename(&self, text: &str, i: usize, new: &str) -> Change {
        let e = &self.entries[i];
        let raw = &text[e.start..e.end];
        if raw.starts_with(&e.key) && !e.key.contains([' ', ':', '"', '\'']) {
            let key = serialize(new, &Value::Null);
            let key = key.trim_end().trim_end_matches(':');
            return Change { start: e.start, end: e.start + e.key.len(), text: key.to_string() };
        }
        Change { start: e.start, end: e.end, text: serialize(new, &e.value) }
    }
    pub fn add(&self, key: &str, v: &Value) -> Change {
        Change { start: self.close, end: self.close, text: serialize(key, v) }
    }
    pub fn index(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.key.eq_ignore_ascii_case(key))
    }
}

/// The change that creates a front matter block holding one property.
pub fn create(key: &str, v: &Value) -> Change {
    Change { start: 0, end: 0, text: format!("---\n{}---\n", serialize(key, v)) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Key(usize),
    Value(usize),
    Item(usize, usize),
    NewItem(usize),
    NewKey,
}

#[derive(Clone, Debug)]
pub struct Edit {
    pub target: Target,
    pub buf: String,
}

fn mode_setting(app: &App) -> &str {
    app.config.app.str("propertiesInDocument").unwrap_or("visible")
}

/// Draw the properties table for the front matter block `text`. Returns false when the block
/// should be shown as source instead.
pub fn render(app: &App, dom: &mut Dom, parent: NodeId, mode: ViewMode, text: &str) -> bool {
    if mode == ViewMode::Source || mode_setting(app) == "source" {
        return false;
    }
    if mode_setting(app) == "hidden" {
        return true;
    }
    let Ok(front) = scan(text) else { return false };
    let edit = app.prop_edit.as_ref();
    let editing = |t: &Target| edit.filter(|e| &e.target == t);
    let shown = |e: &Edit| format!("{}▏", e.buf);
    let wrap = dom.add(parent, El::div().class("metadata-container"));
    dom.add(wrap, El::div().class("metadata-properties-heading").text("Properties"));
    let props = dom.add(wrap, El::div().class("metadata-properties"));
    for (i, en) in front.entries.iter().enumerate() {
        let k = kind(&en.key, &en.value);
        let row = dom.add(props, El::div().class("metadata-property").attr("data-property-key", &en.key));
        let kcell = dom.add(row, El::div().class("metadata-property-key"));
        match editing(&Target::Key(i)) {
            Some(e) => dom.add(kcell, El::div().class("metadata-input is-editing").text(shown(e))),
            None => dom.add(
                kcell,
                El::div().class("metadata-property-key-input").text(en.key.clone()).action(format!("prop:key:{i}")),
            ),
        };
        let val = dom.add(row, El::div().class("metadata-property-value"));
        match k {
            Kind::Checkbox => {
                let on = en.value == Value::Bool(true);
                let b = dom.add(
                    val,
                    El::div().class("checkbox-container").class_if(on, "is-enabled").action(format!("prop:toggle:{i}")),
                );
                dom.add(b, El::div().class("checkbox-thumb"));
            }
            Kind::List => {
                let box_ = dom.add(val, El::div().class("multi-select-container"));
                for (j, item) in list_items(&en.value).iter().enumerate() {
                    let label = item.as_string_list().join(", ");
                    match editing(&Target::Item(i, j)) {
                        Some(e) => dom.add(box_, El::div().class("metadata-input is-editing").text(shown(e))),
                        None => {
                            let pill = dom.add(box_, El::div().class("multi-select-pill"));
                            dom.add(
                                pill,
                                El::div()
                                    .class("multi-select-pill-content")
                                    .text(label)
                                    .action(format!("prop:item:{i}:{j}")),
                            );
                            dom.add(
                                pill,
                                El::div()
                                    .class("multi-select-pill-remove-button")
                                    .text("×")
                                    .action(format!("prop:rm:{i}:{j}")),
                            )
                        }
                    };
                }
                match editing(&Target::NewItem(i)) {
                    Some(e) => dom.add(box_, El::div().class("metadata-input is-editing").text(shown(e))),
                    None => dom.add(
                        box_,
                        El::div().class("multi-select-input").text("+").action(format!("prop:add-item:{i}")),
                    ),
                };
            }
            Kind::Other => {
                let s = serialize("", &en.value);
                let flat = s.trim_start_matches(':').split_whitespace().collect::<Vec<_>>().join(" ");
                dom.add(val, El::div().class("metadata-input-readonly").text(flat));
            }
            Kind::Text | Kind::Number | Kind::Date => {
                let plain = match &en.value {
                    Value::Null => String::new(),
                    Value::String(s) => s.clone(),
                    Value::Number(n) => frontmatter::fmt_number(*n),
                    other => other.as_string_list().join(", "),
                };
                match editing(&Target::Value(i)) {
                    Some(e) => dom.add(val, El::div().class("metadata-input is-editing").text(shown(e))),
                    None if plain.is_empty() => dom.add(
                        val,
                        El::div().class("metadata-input is-empty").text("Empty").action(format!("prop:val:{i}")),
                    ),
                    None => dom.add(
                        val,
                        El::div()
                            .class("metadata-input")
                            .class_if(k == Kind::Number, "metadata-input-number")
                            .class_if(k == Kind::Date, "metadata-input-date")
                            .text(plain)
                            .action(format!("prop:val:{i}")),
                    ),
                };
            }
        }
        dom.add(row, El::div().class("metadata-property-remove").text("×").action(format!("prop:del:{i}")));
    }
    if let Some(e) = editing(&Target::NewKey) {
        let row = dom.add(props, El::div().class("metadata-property"));
        let kcell = dom.add(row, El::div().class("metadata-property-key"));
        let text = if e.buf.is_empty() { "Property name▏".to_string() } else { shown(e) };
        dom.add(kcell, El::div().class("metadata-input is-editing").text(text));
    } else {
        dom.add(wrap, El::div().class("metadata-add-button").text("+ Add property").action("properties:add"));
    }
    true
}

impl App {
    /// The front matter of the active note, from its first block.
    fn active_front(&self) -> Option<(Front, String)> {
        let t = self.tab()?;
        let doc = t.editor.buf.doc();
        let b = doc.blocks.first()?;
        let r = b.tree.root();
        if !matches!(r.kind, NodeKind::FrontMatter { .. }) || b.start != 0 {
            return None;
        }
        let text = t.editor.buf.slice(0, b.end()).into_owned();
        scan(&text).ok().map(|f| (f, text))
    }

    fn apply_prop_change(&mut self, change: Change) {
        let now = fsio::now_ms();
        if let Some(t) = self.tab_mut() {
            t.editor.buf.apply(vec![change], None, EditKind::Other, now);
            t.last_edit_ms = now;
        }
    }

    /// Put the cursor after the front matter so it stays shown as a table.
    pub(crate) fn park_cursor_after_front(&mut self, tab: usize) {
        if mode_setting(self) != "visible" {
            return;
        }
        let Some(t) = self.tabs.get_mut(tab) else { return };
        let doc = t.editor.buf.doc();
        let Some(b) = doc.blocks.first() else { return };
        let r = b.tree.root();
        if matches!(r.kind, NodeKind::FrontMatter { .. }) {
            let at = b.end().min(t.editor.buf.len());
            t.editor.buf.set_sels(vec![Sel::cursor(at)]);
        }
    }

    pub(crate) fn run_properties(&mut self, command: &str) -> bool {
        if let Some(rest) = command.strip_prefix("prop:") {
            self.prop_action(rest);
            return true;
        }
        match command {
            "properties:add" => {
                if self.tab().is_none() {
                    return true;
                }
                if mode_setting(self) != "visible" {
                    self.config.app.set("propertiesInDocument", "visible".into());
                    let _ = self.config.app.save();
                }
                if let Some(t) = self.tab_mut() {
                    if t.mode == ViewMode::Source {
                        t.mode = ViewMode::LivePreview;
                    }
                }
                self.park_cursor_after_front(self.active);
                self.prop_edit = Some(Edit { target: Target::NewKey, buf: String::new() });
            }
            "properties:toggle-source" => {
                let next = if mode_setting(self) == "visible" { "source" } else { "visible" };
                self.config.app.set("propertiesInDocument", next.into());
                if let Err(e) = self.config.app.save() {
                    self.notice = Some(format!("Cannot save app.json: {e}"));
                }
                self.prop_edit = None;
            }
            _ => return false,
        }
        true
    }

    fn prop_action(&mut self, rest: &str) {
        let mut parts = rest.split(':');
        let (op, a, b) = (parts.next().unwrap_or(""), parts.next(), parts.next());
        let i = a.and_then(|v| v.parse::<usize>().ok());
        let j = b.and_then(|v| v.parse::<usize>().ok());
        let starts_edit = matches!(op, "key" | "val" | "item" | "add-item");
        if !starts_edit || self.prop_edit.is_some() {
            self.commit_prop_edit();
        }
        let Some((front, _)) = self.active_front() else { return };
        let Some(i) = i.filter(|&i| i < front.entries.len()) else { return };
        let e = &front.entries[i];
        let plain = |v: &Value| match v {
            Value::Null => String::new(),
            Value::Number(n) => frontmatter::fmt_number(*n),
            other => other.as_string_list().join(", "),
        };
        match op {
            "key" => self.prop_edit = Some(Edit { target: Target::Key(i), buf: e.key.clone() }),
            "val" => self.prop_edit = Some(Edit { target: Target::Value(i), buf: plain(&e.value) }),
            "item" => {
                let items = list_items(&e.value);
                if let Some(item) = j.and_then(|j| items.get(j)) {
                    self.prop_edit = Some(Edit { target: Target::Item(i, j.unwrap_or(0)), buf: plain(item) });
                }
            }
            "add-item" => self.prop_edit = Some(Edit { target: Target::NewItem(i), buf: String::new() }),
            "toggle" => {
                let on = e.value == Value::Bool(true);
                self.apply_prop_change(front.set_value(i, &Value::Bool(!on)));
            }
            "del" => {
                self.prop_edit = None;
                self.apply_prop_change(front.remove(i));
            }
            "rm" => {
                let mut items = list_items(&e.value);
                if let Some(j) = j.filter(|&j| j < items.len()) {
                    items.remove(j);
                    self.apply_prop_change(front.set_value(i, &Value::List(items)));
                }
            }
            _ => {}
        }
    }

    /// Write the text being edited into the note.
    pub(crate) fn commit_prop_edit(&mut self) {
        let Some(edit) = self.prop_edit.take() else { return };
        let value = edit.buf.trim().to_string();
        let found = self.active_front();
        if edit.target == Target::NewKey {
            if value.is_empty() {
                return;
            }
            let initial = if LIST_KEYS.iter().any(|k| k.eq_ignore_ascii_case(&value)) {
                Value::List(Vec::new())
            } else {
                Value::Null
            };
            match &found {
                Some((f, _)) if f.index(&value).is_some() => {
                    self.notice = Some(format!("Property \"{value}\" already exists"));
                    return;
                }
                Some((f, _)) => self.apply_prop_change(f.add(&value, &initial)),
                None => self.apply_prop_change(create(&value, &initial)),
            }
            if let Some((f, _)) = self.active_front() {
                if let Some(i) = f.index(&value) {
                    let target = if initial == Value::Null { Target::Value(i) } else { Target::NewItem(i) };
                    self.prop_edit = Some(Edit { target, buf: String::new() });
                }
            }
            return;
        }
        let Some((front, text)) = found else { return };
        let ok = |i: usize| i < front.entries.len();
        match edit.target {
            Target::Key(i) if ok(i) => {
                if value.is_empty() || value == front.entries[i].key {
                    return;
                }
                if front.index(&value).is_some_and(|x| x != i) {
                    self.notice = Some(format!("Property \"{value}\" already exists"));
                    return;
                }
                self.apply_prop_change(front.rename(&text, i, &value));
            }
            Target::Value(i) if ok(i) => {
                let e = &front.entries[i];
                let v = match kind(&e.key, &e.value) {
                    _ if value.is_empty() => Value::Null,
                    Kind::Number => match value.parse::<f64>() {
                        Ok(n) if n.is_finite() => Value::Number(n),
                        _ => {
                            self.notice = Some(format!("\"{value}\" is not a number"));
                            return;
                        }
                    },
                    _ => Value::String(value),
                };
                if v != e.value {
                    self.apply_prop_change(front.set_value(i, &v));
                }
            }
            Target::Item(i, j) if ok(i) => {
                let mut items = list_items(&front.entries[i].value);
                if j < items.len() {
                    if value.is_empty() {
                        items.remove(j);
                    } else {
                        items[j] = Value::String(value);
                    }
                    self.apply_prop_change(front.set_value(i, &Value::List(items)));
                }
            }
            Target::NewItem(i) if ok(i) => {
                if value.is_empty() {
                    return;
                }
                let mut items = list_items(&front.entries[i].value);
                items.push(Value::String(value));
                self.apply_prop_change(front.set_value(i, &Value::List(items)));
                self.prop_edit = Some(Edit { target: Target::NewItem(i), buf: String::new() });
            }
            _ => {}
        }
    }

    pub(crate) fn prop_key(&mut self, key: &Key) -> Option<bool> {
        let edit = self.prop_edit.as_mut()?;
        if key.ctrl || key.alt {
            return None;
        }
        match key.key.as_str() {
            "Escape" => self.prop_edit = None,
            "Enter" | "Tab" => self.commit_prop_edit(),
            "Backspace" => {
                edit.buf.pop();
            }
            _ => {}
        }
        Some(true)
    }

    pub(crate) fn prop_text(&mut self, text: &str) -> bool {
        match &mut self.prop_edit {
            Some(e) => {
                e.buf.push_str(text);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "---\ntitle: Hello\n# keep me\ntags:\n  - a\n  - b\ncount: 3\ndone: false\ndate: 2024-01-05\n---\n";

    fn apply(text: &str, c: &Change) -> String {
        format!("{}{}{}", &text[..c.start], c.text, &text[c.end..])
    }

    #[test]
    fn scans_entries_and_kinds() {
        let f = scan(DOC).unwrap();
        let keys: Vec<_> = f.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["title", "tags", "count", "done", "date"]);
        assert_eq!(kind("tags", &f.entries[1].value), Kind::List);
        assert_eq!(kind("count", &f.entries[2].value), Kind::Number);
        assert_eq!(kind("done", &f.entries[3].value), Kind::Checkbox);
        assert_eq!(kind("date", &f.entries[4].value), Kind::Date);
        assert_eq!(kind("title", &f.entries[0].value), Kind::Text);
    }

    #[test]
    fn edits_touch_only_their_entry() {
        let f = scan(DOC).unwrap();
        let out = apply(DOC, &f.set_value(0, &Value::String("New: title".into())));
        assert_eq!(out, DOC.replace("title: Hello", "title: \"New: title\""));
        let out = apply(DOC, &f.set_value(1, &Value::List(vec![Value::String("a".into())])));
        assert_eq!(out, DOC.replace("  - b\n", ""));
        let out = apply(DOC, &f.remove(2));
        assert_eq!(out, DOC.replace("count: 3\n", ""));
        let out = apply(DOC, &f.rename(DOC, 2, "total"));
        assert_eq!(out, DOC.replace("count:", "total:"));
        let out = apply(DOC, &f.add("author", &Value::String("Ann".into())));
        assert_eq!(out, DOC.replace("---\n", "---\n").replace("date: 2024-01-05\n", "date: 2024-01-05\nauthor: Ann\n"));
        assert!(out.contains("# keep me"));
    }

    #[test]
    fn creates_front_matter_and_rejects_bad_yaml() {
        assert_eq!(create("a", &Value::Null).text, "---\na:\n---\n");
        assert!(scan("no front matter\n").is_err());
        assert!(scan("---\na: [unclosed\n---\n").is_err());
        assert!(scan("---\na: 1\n").is_err());
        assert!(scan("---\n---\n").unwrap().entries.is_empty());
    }
}
