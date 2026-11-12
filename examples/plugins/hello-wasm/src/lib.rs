//! Example Rustwalnut WASM plugin: no dependencies, no_std.
//! Build: `cargo build --release --target wasm32-unknown-unknown`, then copy
//! `target/wasm32-unknown-unknown/release/hello_wasm.wasm` to `main.wasm` next to `manifest.json`.
#![no_std]
#![allow(static_mut_refs)]

use core::slice;

static mut HEAP: [u8; 64 * 1024] = [0; 64 * 1024];
static mut TOP: usize = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_call(req_ptr: *const u8, req_len: usize, resp_ptr: *mut u8, resp_cap: usize) -> i32;
}

fn call(req: &str) {
    let mut resp = [0u8; 512];
    unsafe {
        host_call(req.as_ptr(), req.len(), resp.as_mut_ptr(), resp.len());
    }
}

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    unsafe {
        if TOP + len > HEAP.len() {
            TOP = 0;
        }
        let p = HEAP.as_mut_ptr().add(TOP);
        TOP += len;
        p
    }
}

#[no_mangle]
pub extern "C" fn on_load() {
    call(r#"{"method":"commands.register","params":{"id":"hello","name":"Say hello"}}"#);
    call(r#"{"method":"commands.register","params":{"id":"stamp","name":"Insert stamp at cursor"}}"#);
    call(r#"{"method":"events.subscribe","params":{"event":"file-open"}}"#);
    call(r#"{"method":"ui.status","params":{"id":"s","text":"hello-wasm"}}"#);
}

#[no_mangle]
pub extern "C" fn on_event(_ptr: *const u8, _len: usize) {
    call(r#"{"method":"ui.status","params":{"id":"s","text":"hello-wasm: file opened"}}"#);
}

#[no_mangle]
pub extern "C" fn run_command(ptr: *const u8, len: usize) -> i32 {
    let id = unsafe { slice::from_raw_parts(ptr, len) };
    match id {
        b"hello" => call(r#"{"method":"ui.notice","params":{"text":"Hello from a WASM plugin"}}"#),
        b"stamp" => call(r#"{"method":"editor.replace","params":{"from":0,"to":0,"text":"stamped\n"}}"#),
        _ => return 1,
    }
    0
}
