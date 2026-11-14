// Release builds on Windows are GUI programs: no console window opens beside the app.
// When started from a terminal, the parent's console is attached so the CLI still prints.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use rustsidian_app::cli::{self, Outcome};

#[cfg(feature = "window")]
mod window;

#[cfg(windows)]
#[allow(unsafe_code)]
mod console {
    use std::os::windows::io::IntoRawHandle;

    type Handle = *mut std::ffi::c_void;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(which: u32) -> Handle;
        fn SetStdHandle(which: u32, handle: Handle) -> i32;
    }

    /// Use the console of the terminal that started this process, if there is one.
    /// Output that was redirected to a file or pipe is left alone.
    pub fn attach() {
        // SAFETY: these are plain Win32 calls with constant arguments. The handle given to
        // `SetStdHandle` is an open file that is intentionally never closed.
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return;
            }
            for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                let current = GetStdHandle(which);
                if current.is_null() || current as isize == -1 {
                    if let Ok(out) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
                        SetStdHandle(which, out.into_raw_handle() as Handle);
                    }
                }
            }
        }
    }
}

fn main() {
    #[cfg(windows)]
    console::attach();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match cli::run(&args, &mut std::io::stdout()) {
        Outcome::Exit(code) => code,
        #[cfg(feature = "window")]
        Outcome::Open { vault, note, profile } => window::run(Some(&vault), note.as_deref(), profile),
        #[cfg(feature = "window")]
        Outcome::Welcome => window::run(None, None, false),
        #[cfg(not(feature = "window"))]
        Outcome::Open { .. } | Outcome::Welcome => {
            eprintln!("rustsidian: this build has no window support (built without the `window` feature)");
            2
        }
    };
    std::process::exit(code);
}
