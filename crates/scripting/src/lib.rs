//! User scripts in Rhai (`.obsidian/scripts/*.rhai`), bound to `ext-api`.
//!
//! The top level of a script runs once when it loads and registers things:
//! `register_command("id", "Name", "fn_name")` and `on("file-open", "fn_name")`. Later the named
//! functions run with the app's [`Api`]. Besides the helpers below, `api("method", #{params})`
//! reaches every method of the JSON call boundary; errors are thrown and can be caught with
//! `try`/`catch`.
//!
//! Helpers: `notice(text)`, `log(text)`, `status(id, text)`, `read(path)`, `write(path, text)`,
//! `list(folder)`, `tags(path)`, `frontmatter(path)`, `active_file()`, `active_text()`,
//! `selection()`, `replace(from, to, text)`, `run(command_id)`, `settings_load()`,
//! `settings_save(map)`.
//!
//! Sandbox: no modules/imports, no file or network functions; operation, call-depth, string, array
//! and map size caps; a wall-clock deadline per run. A script runs on a scoped helper thread while
//! the calling thread serves its API calls, so the language needs no access to the app's state.

use ext_api::{Api, Event, Extension, PluginInfo};
use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, Map, Scope, AST};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_operations: u64,
    pub time_per_run: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_operations: 5_000_000, time_per_run: Duration::from_millis(500) }
    }
}

type Channel = (Sender<String>, Receiver<String>);
type Shared<T> = Arc<Mutex<T>>;

pub struct Script {
    info: PluginInfo,
    engine: Engine,
    ast: AST,
    commands: Shared<HashMap<String, String>>,
    handlers: Shared<HashMap<String, String>>,
    slot: Shared<Option<Channel>>,
    deadline: Shared<Instant>,
    limits: Limits,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Clear<'a>(&'a Shared<Option<Channel>>);

impl Drop for Clear<'_> {
    fn drop(&mut self) {
        *lock(self.0) = None;
    }
}

fn err(msg: impl Into<String>) -> Box<EvalAltResult> {
    msg.into().into()
}

fn request(slot: &Shared<Option<Channel>>, method: &str, params: Value) -> Result<Value, Box<EvalAltResult>> {
    let guard = lock(slot);
    let (tx, rx) = guard.as_ref().ok_or_else(|| err("API is not available here"))?;
    tx.send(json!({"method": method, "params": params}).to_string()).map_err(|_| err("host went away"))?;
    let resp = rx.recv().map_err(|_| err("host went away"))?;
    let mut v: Value = serde_json::from_str(&resp).map_err(|e| err(e.to_string()))?;
    match v.get("error").and_then(Value::as_str) {
        Some(e) => Err(err(e)),
        None => Ok(v["ok"].take()),
    }
}

fn to_dyn(v: Value) -> Result<Dynamic, Box<EvalAltResult>> {
    rhai::serde::to_dynamic(v)
}

impl Script {
    pub fn new(info: PluginInfo, source: &str, limits: Limits) -> Result<Script, String> {
        let slot: Shared<Option<Channel>> = Arc::default();
        let commands: Shared<HashMap<String, String>> = Arc::default();
        let handlers: Shared<HashMap<String, String>> = Arc::default();
        let deadline = Arc::new(Mutex::new(Instant::now()));
        let mut engine = Engine::new();
        engine.set_max_operations(limits.max_operations);
        engine.set_max_call_levels(32);
        engine.set_max_expr_depths(64, 32);
        engine.set_max_string_size(4 << 20);
        engine.set_max_array_size(100_000);
        engine.set_max_map_size(10_000);
        let d = deadline.clone();
        engine.on_progress(move |ops| {
            (ops % 1024 == 0 && Instant::now() > *lock(&d)).then(|| "time limit exceeded".into())
        });
        let s = slot.clone();
        engine.register_fn("api", move |method: &str, params: Dynamic| {
            let p: Value = rhai::serde::from_dynamic(&params)?;
            to_dyn(request(&s, method, p)?)
        });
        macro_rules! helper {
            ($name:literal, $method:literal, || $params:expr) => {{
                let s = slot.clone();
                engine.register_fn($name, move || to_dyn(request(&s, $method, $params)?));
            }};
            ($name:literal, $method:literal, |$($a:ident: $t:ty),*| $params:expr) => {{
                let s = slot.clone();
                engine.register_fn($name, move |$($a: $t),*| to_dyn(request(&s, $method, $params)?));
            }};
        }
        helper!("notice", "ui.notice", |t: &str| json!({"text": t}));
        helper!("log", "log", |t: &str| json!({"text": t}));
        helper!("status", "ui.status", |id: &str, t: &str| json!({"id": id, "text": t}));
        helper!("read", "vault.read", |p: &str| json!({"path": p}));
        helper!("write", "vault.write", |p: &str, t: &str| json!({"path": p, "text": t}));
        helper!("list", "vault.list", |f: &str| json!({"folder": f}));
        helper!("tags", "meta.tags", |p: &str| json!({"path": p}));
        helper!("frontmatter", "meta.frontmatter", |p: &str| json!({"path": p}));
        helper!("active_file", "editor.active_file", || json!({}));
        helper!("active_text", "editor.text", || json!({}));
        helper!("selection", "editor.selection", || json!({}));
        helper!(
            "replace",
            "editor.replace",
            |a: i64, b: i64, t: &str| json!({"from": a, "to": b, "text": t})
        );
        helper!("run", "commands.run", |id: &str| json!({"id": id}));
        helper!("settings_load", "settings.load", || json!({}));
        helper!(
            "settings_save",
            "settings.save",
            |m: Map| json!({"data": rhai::serde::from_dynamic::<Value>(&m.into())?})
        );
        let (s, c) = (slot.clone(), commands.clone());
        engine.register_fn("register_command", move |id: &str, name: &str, func: ImmutableString| {
            request(&s, "commands.register", json!({"id": id, "name": name}))?;
            lock(&c).insert(id.to_string(), func.to_string());
            Ok::<(), Box<EvalAltResult>>(())
        });
        let (s, h) = (slot.clone(), handlers.clone());
        engine.register_fn("on", move |event: &str, func: ImmutableString| {
            request(&s, "events.subscribe", json!({"event": event}))?;
            lock(&h).insert(event.to_string(), func.to_string());
            Ok::<(), Box<EvalAltResult>>(())
        });
        let ast = engine.compile(source).map_err(|e| format!("script does not compile: {e}"))?;
        Ok(Script { info, engine, ast, commands, handlers, slot, deadline, limits })
    }

    /// Run `f` on a helper thread, serving its API calls on this one.
    fn run<F>(&mut self, api: &mut Api, f: F) -> Result<(), String>
    where
        F: FnOnce(&Engine, &AST) -> Result<(), Box<EvalAltResult>> + Send,
    {
        let (rq_tx, rq_rx) = channel::<String>();
        let (rs_tx, rs_rx) = channel::<String>();
        *lock(&self.slot) = Some((rq_tx, rs_rx));
        *lock(&self.deadline) = Instant::now() + self.limits.time_per_run;
        let (engine, ast, slot, info) = (&self.engine, &self.ast, &self.slot, &self.info);
        let result = std::thread::scope(|s| {
            let worker = s.spawn(move || {
                let _clear = Clear(slot);
                f(engine, ast)
            });
            while let Ok(req) = rq_rx.recv() {
                let _ = rs_tx.send(api.call(info, &req));
            }
            worker.join()
        });
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("script panicked".into()),
        }
    }

    fn call_named(&mut self, api: &mut Api, func: String, args: Vec<Dynamic>) -> Result<(), String> {
        self.run(api, move |engine, ast| {
            let opts = rhai::CallFnOptions::new().eval_ast(false).rewind_scope(true);
            engine.call_fn_with_options::<Dynamic>(opts, &mut Scope::new(), ast, &func, args).map(|_| ())
        })
    }
}

impl Extension for Script {
    fn load(&mut self, api: &mut Api) -> Result<(), String> {
        self.run(api, |engine, ast| engine.run_ast(ast))
    }

    fn event(&mut self, event: &Event, api: &mut Api) -> Result<(), String> {
        let Some(func) = lock(&self.handlers).get(event.name()).cloned() else { return Ok(()) };
        let arg = rhai::serde::to_dynamic(serde_json::to_value(event).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        self.call_named(api, func, vec![arg])
    }

    fn command(&mut self, local_id: &str, api: &mut Api) -> Result<(), String> {
        let func = lock(&self.commands)
            .get(local_id)
            .cloned()
            .ok_or_else(|| format!("unknown command {local_id}"))?;
        self.call_named(api, func, Vec::new())
    }
}

pub fn load_file(info: PluginInfo, path: &std::path::Path) -> Result<Script, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Script::new(info, &src, Limits::default())
}
