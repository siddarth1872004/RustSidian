use ext_api::testing::MemHost;
use ext_api::{Event, Extensions, Permissions, PluginInfo};
use scripting::{Limits, Script};
use std::time::{Duration, Instant};

fn info(perms: Permissions) -> PluginInfo {
    PluginInfo {
        id: "script-demo".into(),
        name: "demo".into(),
        perms,
        data_path: ".obsidian/scripts/demo.data.json".into(),
    }
}

fn setup(src: &str, perms: Permissions, limits: Limits) -> (Extensions, MemHost, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut host = MemHost::new(dir.path());
    host.files.insert("a.md".into(), "one #t1 two".into());
    host.files.insert("b.md".into(), "three".into());
    let mut ext = Extensions::default();
    let script = Script::new(info(perms.clone()), src, limits).unwrap();
    ext.add(&mut host, info(perms), Box::new(script)).unwrap();
    (ext, host, dir)
}

const SRC: &str = r#"
register_command("count", "Count notes", "count_notes");
register_command("append", "Append to a", "append_to_a");
register_command("escape", "Try escaping", "escape");
register_command("spin", "Spin", "spin");
on("file-open", "opened");

fn count_notes() {
    let n = list("").len();
    notice("notes: " + n);
}
fn append_to_a() {
    let t = read("a.md");
    write("a.md", t + " appended");
    status("s", "tags: " + tags("a.md").len());
}
fn escape() {
    try { write("../x.md", "x"); notice("wrote"); } catch (e) { notice("blocked: " + e); }
}
fn spin() { loop { } }
fn opened(ev) { notice("opened " + ev.path); }
"#;

#[test]
fn script_registers_commands_and_uses_the_api() {
    let (mut ext, mut host, _d) = setup(SRC, Permissions::all(), Limits::default());
    assert_eq!(ext.reg.commands.len(), 4);
    assert!(ext.run_command(&mut host, "script-demo:count"));
    assert_eq!(ext.reg.take_notices()[0].text, "notes: 2");
    ext.run_command(&mut host, "script-demo:append");
    assert_eq!(host.files["a.md"], "one #t1 two appended");
    assert_eq!(ext.reg.status[0].text, "tags: 1");
    ext.emit(&mut host, &Event::FileOpen { path: "b.md".into() });
    assert_eq!(ext.reg.take_notices()[0].text, "opened b.md");
}

#[test]
fn permissions_and_confinement_apply_to_scripts() {
    let (mut ext, mut host, _d) = setup(SRC, Permissions::all(), Limits::default());
    ext.run_command(&mut host, "script-demo:escape");
    assert!(ext.reg.take_notices()[0].text.starts_with("blocked: "));
    assert!(!host.files.contains_key("../x.md"));
    let (mut ext, mut host, _d) = setup(SRC, Permissions::default_set(), Limits::default());
    ext.run_command(&mut host, "script-demo:append");
    assert_eq!(host.files["a.md"], "one #t1 two");
    assert!(ext.reg.take_notices()[0].text.contains("permission denied"));
}

#[test]
fn runaway_scripts_are_stopped() {
    let limits = Limits { max_operations: 10_000_000_000, time_per_run: Duration::from_millis(100) };
    let (mut ext, mut host, _d) = setup(SRC, Permissions::all(), limits);
    let t = Instant::now();
    ext.run_command(&mut host, "script-demo:spin");
    assert!(t.elapsed() < Duration::from_secs(5));
    let n = ext.reg.take_notices();
    assert!(n[0].text.contains("terminated"), "{n:?}");
    let (mut ext, mut host, _d) = setup(SRC, Permissions::all(), Limits::default());
    ext.run_command(&mut host, "script-demo:spin");
    assert!(ext.reg.take_notices()[0].text.contains("operations"));
    assert!(ext.run_command(&mut host, "script-demo:count"), "still usable");
}

#[test]
fn bad_scripts_report_errors() {
    let p = info(Permissions::all());
    assert!(Script::new(p.clone(), "fn (", Limits::default()).is_err());
    let dir = tempfile::tempdir().unwrap();
    let mut host = MemHost::new(dir.path());
    let mut ext = Extensions::default();
    let s = Script::new(p.clone(), "register_command(1);", Limits::default()).unwrap();
    assert!(ext.add(&mut host, p, Box::new(s)).is_err());
}
