use ext_api::{Api, Event, Extension, Permissions, PluginInfo};
use serde_json::json;
use std::fs;
use walnut_app::App;

struct Demo(PluginInfo);

impl Demo {
    fn call(&self, api: &mut Api, method: &str, params: serde_json::Value) -> serde_json::Value {
        serde_json::from_str(&api.call(&self.0, &json!({"method": method, "params": params}).to_string()))
            .unwrap()
    }
}

impl Extension for Demo {
    fn load(&mut self, api: &mut Api) -> Result<(), String> {
        self.call(
            api,
            "commands.register",
            json!({"id": "tag", "name": "Append tag", "hotkey": "Ctrl+Alt+T"}),
        );
        self.call(api, "commands.register", json!({"id": "mk", "name": "Make note"}));
        self.call(api, "commands.register", json!({"id": "evil", "name": "Escape"}));
        self.call(api, "events.subscribe", json!({"event": "file-open"}));
        self.call(api, "ui.status", json!({"id": "s", "text": "demo ready"}));
        self.call(api, "ui.ribbon", json!({"id": "r", "icon": "D", "title": "Demo", "command": "tag"}));
        Ok(())
    }
    fn event(&mut self, e: &Event, api: &mut Api) -> Result<(), String> {
        if let Event::FileOpen { path } = e {
            self.call(api, "ui.status", json!({"id": "s", "text": format!("opened {path}")}));
        }
        Ok(())
    }
    fn command(&mut self, id: &str, api: &mut Api) -> Result<(), String> {
        match id {
            "tag" => {
                let t = self.call(api, "editor.text", json!({}))["ok"].as_str().unwrap().len();
                self.call(api, "editor.replace", json!({"from": t, "to": t, "text": " #fromplugin"}));
            }
            "mk" => {
                self.call(api, "vault.write", json!({"path": "Made.md", "text": "[[Home]] #made"}));
            }
            _ => {
                let r = self.call(api, "vault.write", json!({"path": "../evil.md", "text": "x"}));
                assert!(r.get("error").is_some());
            }
        }
        Ok(())
    }
}

fn setup() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Home.md"), "# Home\n").unwrap();
    fs::create_dir_all(d.path().join(".obsidian/plugins/wasm-x")).unwrap();
    fs::write(
        d.path().join(".obsidian/plugins/wasm-x/manifest.json"),
        r#"{"id":"wasm-x","name":"X","version":"1"}"#,
    )
    .unwrap();
    fs::write(d.path().join(".obsidian/plugins/wasm-x/main.wasm"), b"junk").unwrap();
    fs::write(d.path().join(".obsidian/community-plugins.json"), r#"["wasm-x"]"#).unwrap();
    let app = App::open(d.path()).unwrap();
    (d, app)
}

#[test]
fn plugin_commands_editor_vault_events_and_ui() {
    let (d, mut app) = setup();
    let info = PluginInfo {
        id: "demo".into(),
        name: "Demo".into(),
        perms: Permissions::all(),
        data_path: ".obsidian/plugins/demo/data.json".into(),
    };
    app.add_extension(info.clone(), Box::new(Demo(info))).unwrap();
    app.open_note("Home");
    app.run("demo:tag");
    assert!(app.tab().unwrap().editor.buf.text().ends_with(" #fromplugin"));
    app.run("demo:mk");
    assert_eq!(fs::read_to_string(d.path().join("Made.md")).unwrap(), "[[Home]] #made");
    assert!(app.store.index.id("Made.md").is_some());
    app.run("demo:evil");
    assert!(!d.path().parent().unwrap().join("evil.md").exists());
    app.pump_plugins();
    assert!(app.plugins.ext.reg.status.iter().any(|s| s.text == "opened Home.md"));
    assert!(app.plugin_palette().iter().any(|(id, _)| id == "demo:tag"));
    assert!(app.keymap.iter().any(|(_, id)| id == "demo:tag"));
}

#[test]
fn plugin_list_and_reload_report_unloadable_plugins() {
    let (_d, mut app) = setup();
    let list = app.plugin_list();
    assert_eq!(list.len(), 1);
    assert!(list[0].enabled && !list[0].loaded);
    app.run("plugins:reload");
    let list = app.plugin_list();
    assert!(list[0].error.is_some(), "junk wasm must report an error, not crash");
    app.run("plugins:list");
    assert!(app.notice.as_deref().unwrap().contains("wasm-x"));
    app.set_plugin_enabled("wasm-x", false).unwrap();
    app.run("plugins:reload");
    assert!(!app.plugin_list()[0].enabled);
}
