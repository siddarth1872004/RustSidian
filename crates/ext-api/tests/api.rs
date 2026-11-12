use ext_api::testing::MemHost;
use ext_api::{Api, Event, Extension, Extensions, Permissions, PluginInfo, Registry};
use serde_json::{json, Value};

fn info(perms: Permissions) -> PluginInfo {
    PluginInfo {
        id: "demo".into(),
        name: "Demo".into(),
        perms,
        data_path: ".obsidian/plugins/demo/data.json".into(),
    }
}

fn call(api: &mut Api, p: &PluginInfo, method: &str, params: Value) -> Value {
    serde_json::from_str(&api.call(p, &json!({"method": method, "params": params}).to_string())).unwrap()
}

#[test]
fn vault_access_is_permissioned_and_confined() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = MemHost::new(dir.path());
    host.files.insert("a.md".into(), "hello #x".into());
    let mut reg = Registry::default();
    let mut api = Api { reg: &mut reg, host: &mut host };
    let ro = info(Permissions::default_set());
    assert_eq!(call(&mut api, &ro, "vault.read", json!({"path": "a.md"})), json!({"ok": "hello #x"}));
    assert_eq!(call(&mut api, &ro, "meta.tags", json!({"path": "a.md"})), json!({"ok": ["x"]}));
    let denied = call(&mut api, &ro, "vault.write", json!({"path": "b.md", "text": "x"}));
    assert!(denied["error"].as_str().unwrap().contains("permission denied"));
    for bad in ["../outside.md", "/etc/passwd", ".obsidian/app.json", "a/../../x.md"] {
        let r = call(&mut api, &info(Permissions::all()), "vault.write", json!({"path": bad, "text": "x"}));
        assert!(r.get("error").is_some(), "{bad} must be rejected");
        let r = call(&mut api, &info(Permissions::all()), "vault.read", json!({"path": bad}));
        assert!(r.get("error").is_some(), "{bad} must be rejected");
    }
    let rw = info(Permissions::all());
    assert_eq!(call(&mut api, &rw, "vault.write", json!({"path": "b.md", "text": "x"}))["ok"], Value::Null);
    assert_eq!(call(&mut api, &rw, "vault.rename", json!({"from": "b.md", "to": "c.md"}))["ok"], Value::Null);
    assert_eq!(call(&mut api, &rw, "vault.list", json!({"ext": "md"})), json!({"ok": ["a.md", "c.md"]}));
    assert!(call(&mut api, &rw, "nope", json!({})).get("error").is_some());
    assert!(api.call(&rw, "not json").contains("bad request"));
}

#[test]
fn editor_ui_commands_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = MemHost::new(dir.path());
    host.active = Some("a.md".into());
    host.editor = "hello world".into();
    host.sel = (6, 11);
    let mut reg = Registry::default();
    let mut api = Api { reg: &mut reg, host: &mut host };
    let p = info(Permissions::all());
    assert_eq!(call(&mut api, &p, "editor.selection", json!({}))["ok"]["text"], "world");
    call(&mut api, &p, "editor.replace", json!({"from": 6, "to": 11, "text": "there"}));
    assert_eq!(call(&mut api, &p, "editor.text", json!({}))["ok"], "hello there");
    assert!(call(&mut api, &p, "editor.replace", json!({"from": 6, "to": 99, "text": ""}))
        .get("error")
        .is_some());
    assert_eq!(call(&mut api, &p, "commands.register", json!({"id": "go", "name": "Go"}))["ok"], "demo:go");
    assert!(call(&mut api, &p, "commands.register", json!({"id": "a b", "name": "Go"}))
        .get("error")
        .is_some());
    call(&mut api, &p, "ui.notice", json!({"text": "hi"}));
    call(&mut api, &p, "ui.status", json!({"id": "s", "text": "one"}));
    call(&mut api, &p, "ui.status", json!({"id": "s", "text": "two"}));
    call(&mut api, &p, "ui.ribbon", json!({"id": "r", "icon": "*", "title": "t", "command": "go"}));
    call(&mut api, &p, "settings.save", json!({"data": {"n": 3}}));
    assert_eq!(call(&mut api, &p, "settings.load", json!({}))["ok"], json!({"n": 3}));
    assert!(dir.path().join(".obsidian/plugins/demo/data.json").is_file());
    assert_eq!(reg.status.len(), 1);
    assert_eq!(reg.status[0].text, "two");
    assert_eq!(reg.ribbon[0].command, "demo:go");
    assert_eq!(reg.notices[0].text, "hi");
    assert_eq!(reg.commands[0].name, "Demo: Go");
}

struct Echo {
    info: PluginInfo,
    fail: bool,
}

impl Extension for Echo {
    fn load(&mut self, api: &mut Api) -> Result<(), String> {
        api.call(&self.info, r#"{"method":"commands.register","params":{"id":"go","name":"Go"}}"#);
        api.call(&self.info, r#"{"method":"events.subscribe","params":{"event":"modify"}}"#);
        Ok(())
    }
    fn event(&mut self, event: &Event, api: &mut Api) -> Result<(), String> {
        if self.fail {
            return Err("boom".into());
        }
        api.call(
            &self.info,
            &json!({"method": "ui.notice", "params": {"text": event.to_json()}}).to_string(),
        );
        Ok(())
    }
    fn command(&mut self, local_id: &str, _api: &mut Api) -> Result<(), String> {
        if local_id == "go" && !self.fail {
            Ok(())
        } else {
            Err("bad".into())
        }
    }
}

#[test]
fn manager_routes_and_disables_failing_plugins() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = MemHost::new(dir.path());
    let mut ext = Extensions::default();
    ext.add(
        &mut host,
        info(Permissions::all()),
        Box::new(Echo { info: info(Permissions::all()), fail: false }),
    )
    .unwrap();
    assert!(ext.run_command(&mut host, "demo:go"));
    assert!(!ext.run_command(&mut host, "demo:missing"));
    ext.emit(&mut host, &Event::Modify { path: "a.md".into() });
    ext.emit(&mut host, &Event::FileOpen { path: "a.md".into() });
    let notices = ext.reg.take_notices();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].text.contains("\"modify\""));
    ext.unload_all(&mut host);
    assert!(ext.reg.commands.is_empty());

    let mut ext = Extensions::default();
    ext.add(
        &mut host,
        info(Permissions::all()),
        Box::new(Echo { info: info(Permissions::all()), fail: true }),
    )
    .unwrap();
    for _ in 0..5 {
        ext.emit(&mut host, &Event::Modify { path: "a.md".into() });
    }
    assert_eq!(ext.loaded().count(), 0);
    assert!(ext.reg.commands.is_empty());
}
