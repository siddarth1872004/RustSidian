use serde_json::json;
use std::fs;
use walnut_app::{App, Key};

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |p: &str, t: &str| {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    };
    w("Home.md", "# Home\n");
    w(".obsidian/hotkeys.json", r#"{"editor:toggle-highlight":[{"modifiers":["Mod","Shift"],"key":"H"}]}"#);
    w(".obsidian/appearance.json", r#"{"theme":"moonstone","enabledCssSnippets":["s"]}"#);
    w(".obsidian/snippets/s.css", ".theme-light { --background-primary: #fefefe }");
    d
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.text_input(&c.to_string());
    }
}

fn read_json(d: &tempfile::TempDir, p: &str) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(d.path().join(p)).unwrap()).unwrap()
}

#[test]
fn settings_persist_and_keep_unknown_keys() {
    let d = vault();
    fs::write(d.path().join(".obsidian/app.json"), r#"{"foo":{"x":1},"vimMode":false}"#).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    assert!(app.settings.is_some());
    app.run("settings:toggle:vimMode");
    assert!(app.vim);
    app.run("settings:cycle:newLinkFormat");
    app.run("settings:edit:tabSize");
    app.key(key("Backspace"));
    typed(&mut app, "2");
    app.key(key("Enter"));
    let j = read_json(&d, ".obsidian/app.json");
    assert_eq!(j["foo"]["x"], 1);
    assert_eq!(j["vimMode"], true);
    assert_eq!(j["newLinkFormat"], "relative");
    assert_eq!(j["tabSize"], 2);
    app.run("settings:section:appearance");
    app.run("settings:cycle:theme");
    app.run("settings:snippet:other");
    let a = read_json(&d, ".obsidian/appearance.json");
    assert_eq!(a["theme"], "system");
    assert_eq!(a["enabledCssSnippets"], json!(["s", "other"]));
    app.key(key("Escape"));
    assert!(app.settings.is_none());
}

#[test]
fn hotkey_rebind_writes_obsidian_format() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("app:open-settings");
    app.run("settings:section:hotkeys");
    app.run("settings:hk:editor:toggle-code");
    app.key(key("Ctrl+Shift+K"));
    let j = read_json(&d, ".obsidian/hotkeys.json");
    assert_eq!(j["editor:toggle-code"], json!([{"modifiers":["Mod","Shift"],"key":"K"}]));
    assert!(j.get("editor:toggle-highlight").is_some());
    assert_eq!(app.hotkey_for("editor:toggle-code"), "Ctrl+Shift+K");
    app.run("settings:hk-reset:editor:toggle-code");
    assert!(read_json(&d, ".obsidian/hotkeys.json").get("editor:toggle-code").is_none());
}

#[test]
fn properties_edit_through_buffer_with_undo() {
    let d = vault();
    fs::write(d.path().join("P.md"), "---\ntitle: A\ntags:\n  - x\ndone: false\n---\nBody\n").unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("P");
    let text = |a: &App| a.tab().unwrap().editor.buf.text();
    app.run("prop:toggle:2");
    assert!(text(&app).contains("done: true"));
    app.run("prop:add-item:1");
    typed(&mut app, "y");
    app.key(key("Enter"));
    app.key(key("Escape"));
    assert!(text(&app).contains("  - x\n  - y\n"));
    app.run("prop:val:0");
    app.key(key("Backspace"));
    typed(&mut app, "B: c");
    app.key(key("Enter"));
    assert!(text(&app).contains("title: \"B: c\"\n"));
    app.run("properties:add");
    typed(&mut app, "n");
    app.key(key("Enter"));
    typed(&mut app, "five");
    app.key(key("Enter"));
    assert!(text(&app).contains("n: five\n---"));
    app.run("prop:del:0");
    assert!(!text(&app).contains("title"));
    assert!(text(&app).ends_with("Body\n"));
    app.run("editor:undo");
    assert!(text(&app).contains("title: \"B: c\""));
    app.save();
    assert!(fs::read_to_string(d.path().join("P.md")).unwrap().contains("done: true"));
}

#[test]
fn new_note_location_setting() {
    let d = vault();
    fs::write(
        d.path().join(".obsidian/app.json"),
        r#"{"newFileLocation":"folder","newFileFolderPath":"Inbox"}"#,
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.run("file-explorer:new-file");
    assert_eq!(app.tab().unwrap().path, "Inbox/Untitled.md");
}
