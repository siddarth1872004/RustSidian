use ext_api::testing::MemHost;
use ext_api::{Event, Extensions, Permissions, PluginInfo};
use plugin_host_wasm::{Limits, WasmPlugin};
use std::time::{Duration, Instant};

const REGISTER: &str = r#"{"method":"commands.register","params":{"id":"hi","name":"Say hi"}}"#;
const SUBSCRIBE: &str = r#"{"method":"events.subscribe","params":{"event":"file-open"}}"#;
const NOTICE: &str = r#"{"method":"ui.notice","params":{"text":"hello from wasm"}}"#;
const ESCAPE: &str = r#"{"method":"vault.write","params":{"path":"../escape.md","text":"x"}}"#;
const WRITE: &str = r#"{"method":"vault.write","params":{"path":"made.md","text":"from wasm"}}"#;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn module() -> Vec<u8> {
    let extra: Vec<String> = ["l", "g", "f", "p", "w", "t"]
        .iter()
        .map(|c| format!(r#"{{"method":"commands.register","params":{{"id":"{c}","name":"{c}"}}}}"#))
        .collect();
    let mut strings = vec![REGISTER, SUBSCRIBE, NOTICE, ESCAPE, WRITE];
    strings.extend(extra.iter().map(String::as_str));
    let mut data = String::new();
    let mut calls = Vec::new();
    for (i, s) in strings.iter().enumerate() {
        let off = 1024 + i * 256;
        data.push_str(&format!("(data (i32.const {off}) \"{}\")\n", esc(s)));
        calls.push((off, s.len()));
    }
    let call = |i: usize| {
        format!(
            "(drop (call $hc (i32.const {}) (i32.const {}) (i32.const 16384) (i32.const 4096)))",
            calls[i].0, calls[i].1
        )
    };
    let src = format!(
        r#"(module
  (import "env" "host_call" (func $hc (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 4)
  (global $heap (mut i32) (i32.const 32768))
  (func (export "alloc") (param $n i32) (result i32) (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))
  {data}
  (func (export "on_load") {register} {subscribe} {more})
  (func (export "on_event") (param $p i32) (param $l i32) {notice})
  (func (export "run_command") (param $p i32) (param $l i32) (result i32)
    (local $c i32)
    (local.set $c (i32.load8_u (local.get $p)))
    ;; 'h' hello
    (if (i32.eq (local.get $c) (i32.const 104)) (then {notice} (return (i32.const 0))))
    ;; 'l' loop forever
    (if (i32.eq (local.get $c) (i32.const 108)) (then (loop $l (local.set $c (i32.add (local.get $c) (i32.const 1))) (br $l))))
    ;; 'g' grow memory until refused
    (if (i32.eq (local.get $c) (i32.const 103)) (then
      (loop $g
        (if (i32.ge_s (memory.grow (i32.const 1024)) (i32.const 0)) (then (br $g))))
      (return (i32.const 7))))
    ;; 'f' flood the host
    (if (i32.eq (local.get $c) (i32.const 102)) (then (loop $f {notice} (br $f))))
    ;; 'p' path traversal
    (if (i32.eq (local.get $c) (i32.const 112)) (then {escape} (return (i32.const 0))))
    ;; 'w' write a note
    (if (i32.eq (local.get $c) (i32.const 119)) (then {write} (return (i32.const 0))))
    ;; 't' trap
    (if (i32.eq (local.get $c) (i32.const 116)) (then unreachable))
    (i32.const 1))
)"#,
        register = call(0),
        subscribe = call(1),
        notice = call(2),
        escape = call(3),
        write = call(4),
        more = (5..strings.len()).map(call).collect::<Vec<_>>().join(" "),
    );
    wat::parse_str(src).unwrap()
}

fn info(perms: Permissions) -> PluginInfo {
    PluginInfo {
        id: "wasm-demo".into(),
        name: "Wasm Demo".into(),
        perms,
        data_path: ".obsidian/plugins/wasm-demo/data.json".into(),
    }
}

fn setup(perms: Permissions, limits: Limits) -> (Extensions, MemHost, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut host = MemHost::new(dir.path());
    let mut ext = Extensions::default();
    let plugin = WasmPlugin::new(info(perms.clone()), &module(), limits).unwrap();
    ext.add(&mut host, info(perms), Box::new(plugin)).unwrap();
    (ext, host, dir)
}

#[test]
fn loads_registers_runs_and_receives_events() {
    let (mut ext, mut host, _d) = setup(Permissions::default_set(), Limits::default());
    assert_eq!(ext.reg.commands[0].id, "wasm-demo:hi");
    assert!(ext.run_command(&mut host, "wasm-demo:hi"));
    assert_eq!(ext.reg.take_notices()[0].text, "hello from wasm");
    ext.emit(&mut host, &Event::FileOpen { path: "a.md".into() });
    assert_eq!(ext.reg.take_notices().len(), 1);
    ext.emit(&mut host, &Event::Modify { path: "a.md".into() });
    assert!(ext.reg.take_notices().is_empty());
}

#[test]
fn permissions_and_path_confinement_hold_inside_wasm() {
    let (mut ext, mut host, _d) = setup(Permissions::all(), Limits::default());
    ext.run_command(&mut host, "wasm-demo:p");
    assert!(host.files.is_empty(), "traversal write must not reach the host");
    ext.run_command(&mut host, "wasm-demo:w");
    assert_eq!(host.files.get("made.md").map(String::as_str), Some("from wasm"));

    let (mut ext, mut host, _d) = setup(Permissions::default_set(), Limits::default());
    ext.run_command(&mut host, "wasm-demo:w");
    assert!(host.files.is_empty(), "no vault.write permission");
}

#[test]
fn infinite_loop_is_stopped_by_fuel_and_app_survives() {
    let limits =
        Limits { fuel_per_call: 20_000_000, time_per_call: Duration::from_secs(5), ..Limits::default() };
    let (mut ext, mut host, _d) = setup(Permissions::default_set(), limits);
    let t = Instant::now();
    assert!(ext.run_command(&mut host, "wasm-demo:l"));
    assert!(t.elapsed() < Duration::from_secs(5));
    let n = ext.reg.take_notices();
    assert!(n[0].text.contains("fuel limit"), "{n:?}");
    assert!(ext.run_command(&mut host, "wasm-demo:hi"), "plugin still works after a stopped call");
}

#[test]
fn deadline_is_checked_at_host_calls() {
    let limits = Limits {
        time_per_call: Duration::from_millis(30),
        host_calls_per_call: u32::MAX,
        ..Limits::default()
    };
    let (mut ext, mut host, _d) = setup(Permissions::default_set(), limits);
    ext.run_command(&mut host, "wasm-demo:f");
    assert!(ext.reg.take_notices().last().unwrap().text.contains("time limit"));
}

#[test]
fn memory_is_capped() {
    let limits = Limits { memory_bytes: 8 << 20, ..Limits::default() };
    let (mut ext, mut host, _d) = setup(Permissions::default_set(), limits);
    ext.run_command(&mut host, "wasm-demo:g");
    let n = ext.reg.take_notices();
    assert!(n[0].text.contains("returned 7"), "grow was refused at the cap: {n:?}");
}

#[test]
fn host_call_flood_and_traps_are_contained() {
    let limits = Limits { host_calls_per_call: 100, ..Limits::default() };
    let (mut ext, mut host, _d) = setup(Permissions::default_set(), limits);
    ext.run_command(&mut host, "wasm-demo:f");
    assert!(ext.reg.take_notices().iter().any(|n| n.text.contains("host call limit")));
    ext.run_command(&mut host, "wasm-demo:t");
    assert!(ext.reg.take_notices()[0].text.contains("failed"));
}

#[test]
fn rejects_bad_modules() {
    let p = info(Permissions::default_set());
    assert!(WasmPlugin::new(p.clone(), b"not wasm", Limits::default()).is_err());
    let no_alloc = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
    assert!(WasmPlugin::new(p.clone(), &no_alloc, Limits::default()).err().unwrap().contains("alloc"));
    let big = Limits { max_module_bytes: 10, ..Limits::default() };
    assert!(WasmPlugin::new(p, &module(), big).is_err());
}
