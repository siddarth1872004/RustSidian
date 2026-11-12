//! WASM plugin host.
//!
//! ABI (all pointers are offsets into the guest's exported `memory`, strings are UTF-8 JSON):
//!
//! guest exports
//! - `memory`
//! - `alloc(len: i32) -> i32`: the host writes events and command ids into memory it gets here
//! - `on_load()`: optional
//! - `on_event(ptr: i32, len: i32)`: optional, receives an `ext_api::Event` as JSON
//! - `run_command(ptr: i32, len: i32) -> i32`: receives the local command id; non-zero is failure
//! - `on_unload()`: optional
//!
//! host import `env.host_call(req_ptr, req_len, resp_ptr, resp_cap) -> i32`
//! - Sends the JSON request `{"method", "params"}` and gets `{"ok"}` or `{"error"}` back.
//! - Returns the response length. If it is larger than `resp_cap` nothing is written; the guest
//!   allocates that many bytes and calls `host_call(0, 0, new_ptr, new_cap)` to fetch the same
//!   response. Returns a negative value for a malformed pointer or an oversized request.
//!
//! Limits: every entry call gets a fuel budget (this is what stops a loop that never calls the
//! host) and a wall-clock deadline checked at every host call, memory is capped, and a call
//! may make a bounded number of host calls. A plugin that exceeds any limit is trapped and
//! the app carries on.

use ext_api::{Api, Event, Extension};
use std::time::{Duration, Instant};
use wasmi::{
    Caller, Engine, Error, Func, Instance, Linker, Memory, Module, ResumableCall, Store, StoreLimits,
    StoreLimitsBuilder, Val,
};

#[derive(Clone, Debug)]
pub struct Limits {
    pub fuel_per_call: u64,
    pub time_per_call: Duration,
    pub memory_bytes: usize,
    pub host_calls_per_call: u32,
    pub max_module_bytes: usize,
    pub max_request_bytes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            fuel_per_call: 100_000_000,
            time_per_call: Duration::from_millis(500),
            memory_bytes: 32 << 20,
            host_calls_per_call: 10_000,
            max_module_bytes: 8 << 20,
            max_request_bytes: 4 << 20,
        }
    }
}

struct HostCall {
    req_ptr: i32,
    req_len: i32,
    resp_ptr: i32,
    resp_cap: i32,
}

struct Data {
    limits: StoreLimits,
    pending: Option<HostCall>,
    last_response: Vec<u8>,
}

#[derive(Debug)]
struct Suspend;

impl std::fmt::Display for Suspend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("host call")
    }
}

impl std::error::Error for Suspend {}

impl wasmi::errors::HostError for Suspend {}

pub struct WasmPlugin {
    info: ext_api::PluginInfo,
    store: Store<Data>,
    instance: Instance,
    memory: Memory,
    limits: Limits,
}

impl WasmPlugin {
    pub fn new(info: ext_api::PluginInfo, wasm: &[u8], limits: Limits) -> Result<WasmPlugin, String> {
        if wasm.len() > limits.max_module_bytes {
            return Err(format!("module is {} bytes, limit is {}", wasm.len(), limits.max_module_bytes));
        }
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| format!("invalid module: {e}"))?;
        let std_limits = StoreLimitsBuilder::new()
            .memory_size(limits.memory_bytes)
            .memories(1)
            .tables(4)
            .table_elements(100_000)
            .instances(1)
            .build();
        let mut store =
            Store::new(&engine, Data { limits: std_limits, pending: None, last_response: Vec::new() });
        store.limiter(|d| &mut d.limits);
        let mut linker = <Linker<Data>>::new(&engine);
        linker
            .func_wrap(
                "env",
                "host_call",
                |mut c: Caller<'_, Data>,
                 req_ptr: i32,
                 req_len: i32,
                 resp_ptr: i32,
                 resp_cap: i32|
                 -> Result<i32, Error> {
                    if req_len == 0 {
                        let resp = std::mem::take(&mut c.data_mut().last_response);
                        let n = resp.len();
                        if n as i64 > i64::from(resp_cap) {
                            c.data_mut().last_response = resp;
                            return Ok(-1);
                        }
                        let Some(mem) = c.get_export("memory").and_then(|e| e.into_memory()) else {
                            return Ok(-1);
                        };
                        return Ok(if mem.write(&mut c, resp_ptr as u32 as usize, &resp).is_ok() {
                            n as i32
                        } else {
                            -1
                        });
                    }
                    c.data_mut().pending = Some(HostCall { req_ptr, req_len, resp_ptr, resp_cap });
                    Err(Error::host(Suspend))
                },
            )
            .map_err(|e| e.to_string())?;
        store.set_fuel(limits.fuel_per_call).map_err(|e| e.to_string())?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("instantiation failed: {e}"))?;
        let memory = instance.get_memory(&store, "memory").ok_or("module does not export `memory`")?;
        if instance.get_func(&store, "alloc").is_none() {
            return Err("module does not export `alloc`".into());
        }
        Ok(WasmPlugin { info, store, instance, memory, limits })
    }

    fn export(&self, name: &str) -> Option<Func> {
        self.instance.get_func(&self.store, name)
    }

    fn write_guest(&mut self, bytes: &[u8]) -> Result<(i32, i32), String> {
        let alloc = self.export("alloc").ok_or("no alloc")?;
        let mut out = [Val::I32(0)];
        self.invoke(&alloc, &[Val::I32(bytes.len() as i32)], &mut out, None)?;
        let Val::I32(ptr) = out[0] else { return Err("alloc returned a non-i32".into()) };
        self.memory
            .write(&mut self.store, ptr as u32 as usize, bytes)
            .map_err(|_| "alloc returned an invalid pointer")?;
        Ok((ptr, bytes.len() as i32))
    }

    /// Run a guest function to completion under the limits, serving `host_call`s through `api`.
    fn invoke(
        &mut self,
        func: &Func,
        inputs: &[Val],
        outputs: &mut [Val],
        mut api: Option<&mut Api>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + self.limits.time_per_call;
        let mut calls = 0u32;
        self.store.set_fuel(self.limits.fuel_per_call).map_err(|e| e.to_string())?;
        let mut state = func.call_resumable(&mut self.store, inputs, outputs).map_err(|e| e.to_string())?;
        loop {
            match state {
                ResumableCall::Finished => return Ok(()),
                ResumableCall::OutOfFuel(_) => return Err("fuel limit exceeded".into()),
                ResumableCall::HostTrap(inv) => {
                    if inv.host_error().downcast_ref::<Suspend>().is_none() {
                        return Err(inv.host_error().to_string());
                    }
                    calls += 1;
                    if calls > self.limits.host_calls_per_call {
                        return Err("host call limit exceeded".into());
                    }
                    if Instant::now() > deadline {
                        return Err("time limit exceeded".into());
                    }
                    let ret = self.serve_host_call(api.as_deref_mut());
                    state =
                        inv.resume(&mut self.store, &[Val::I32(ret)], outputs).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    fn serve_host_call(&mut self, api: Option<&mut Api>) -> i32 {
        let Some(call) = self.store.data_mut().pending.take() else { return -1 };
        if call.req_len < 0 || call.req_len as usize > self.limits.max_request_bytes || call.resp_cap < 0 {
            return -1;
        }
        let mut req = vec![0u8; call.req_len as usize];
        if self.memory.read(&self.store, call.req_ptr as u32 as usize, &mut req).is_err() {
            return -1;
        }
        let response = match (api, std::str::from_utf8(&req)) {
            (Some(api), Ok(text)) => api.call(&self.info, text),
            (None, _) => r#"{"error":"host calls are not available during alloc"}"#.to_string(),
            (_, Err(_)) => r#"{"error":"request is not UTF-8"}"#.to_string(),
        };
        let bytes = response.into_bytes();
        let n = bytes.len();
        if n <= call.resp_cap as usize {
            if self.memory.write(&mut self.store, call.resp_ptr as u32 as usize, &bytes).is_err() {
                return -1;
            }
        } else {
            self.store.data_mut().last_response = bytes;
        }
        n as i32
    }

    fn call_optional(&mut self, name: &str, args: &[u8], api: &mut Api) -> Result<Option<i32>, String> {
        let Some(func) = self.export(name) else { return Ok(None) };
        let mut inputs = Vec::new();
        if !args.is_empty() || name == "run_command" || name == "on_event" {
            let (ptr, len) = self.write_guest(args)?;
            inputs.extend([Val::I32(ptr), Val::I32(len)]);
        }
        let mut out = vec![Val::I32(0); func.ty(&self.store).results().len()];
        self.invoke(&func, &inputs, &mut out, Some(api))?;
        Ok(match out.first() {
            Some(Val::I32(v)) => Some(*v),
            _ => Some(0),
        })
    }
}

impl Extension for WasmPlugin {
    fn load(&mut self, api: &mut Api) -> Result<(), String> {
        self.call_optional("on_load", &[], api).map(|_| ())
    }

    fn event(&mut self, event: &Event, api: &mut Api) -> Result<(), String> {
        self.call_optional("on_event", event.to_json().as_bytes(), api).map(|_| ())
    }

    fn command(&mut self, local_id: &str, api: &mut Api) -> Result<(), String> {
        match self.call_optional("run_command", local_id.as_bytes(), api)? {
            Some(0) | None => Ok(()),
            Some(code) => Err(format!("command returned {code}")),
        }
    }

    fn unload(&mut self, api: &mut Api) {
        let _ = self.call_optional("on_unload", &[], api);
    }
}

/// Convenience for the app: read `main.wasm` and build the plugin.
pub fn load_file(info: ext_api::PluginInfo, path: &std::path::Path) -> Result<WasmPlugin, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    WasmPlugin::new(info, &bytes, Limits::default())
}
