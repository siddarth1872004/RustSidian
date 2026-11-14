//! The system clipboard, as plain text. Windows is reached through the Win32 API; macOS and Linux
//! through the standard command-line tools (`pbcopy`, `wl-copy`, `xclip`, `xsel`), so no
//! windowing-toolkit clipboard crate is needed.

/// Put text on the system clipboard.
pub fn set(text: &str) -> Result<(), String> {
    imp::set(text)
}

/// Read text from the system clipboard. Line endings come back as `\n`.
pub fn get() -> Result<String, String> {
    imp::get().map(|t| t.replace("\r\n", "\n"))
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;

    type Handle = *mut c_void;
    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 2;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> Handle;
        fn SetClipboardData(format: u32, mem: Handle) -> Handle;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(mem: Handle) -> *mut c_void;
        fn GlobalUnlock(mem: Handle) -> i32;
        fn GlobalFree(mem: Handle) -> Handle;
    }

    /// Another program may hold the clipboard for a moment, so opening is retried briefly.
    fn open() -> Result<(), String> {
        for _ in 0..10 {
            // SAFETY: no pointers are passed; a null owner is allowed.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err("the clipboard is in use by another program".into())
    }

    pub fn set(text: &str) -> Result<(), String> {
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
        open()?;
        // SAFETY: the clipboard is open. The block is allocated with the size of `wide` and only
        // written while locked; after SetClipboardData succeeds the system owns it.
        let result = unsafe {
            EmptyClipboard();
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
            if mem.is_null() {
                Err("out of memory".to_string())
            } else {
                let dst = GlobalLock(mem) as *mut u16;
                if dst.is_null() {
                    GlobalFree(mem);
                    Err("cannot lock clipboard memory".to_string())
                } else {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                    GlobalUnlock(mem);
                    if SetClipboardData(CF_UNICODETEXT, mem).is_null() {
                        GlobalFree(mem);
                        Err("cannot set clipboard data".to_string())
                    } else {
                        Ok(())
                    }
                }
            }
        };
        // SAFETY: opened above.
        unsafe { CloseClipboard() };
        result
    }

    pub fn get() -> Result<String, String> {
        open()?;
        // SAFETY: the clipboard is open; the handle is only read while locked, up to the
        // terminating zero that CF_UNICODETEXT data always has.
        let result = unsafe {
            let mem = GetClipboardData(CF_UNICODETEXT);
            if mem.is_null() {
                Ok(String::new())
            } else {
                let src = GlobalLock(mem) as *const u16;
                if src.is_null() {
                    Err("cannot lock clipboard memory".to_string())
                } else {
                    let mut len = 0;
                    while *src.add(len) != 0 {
                        len += 1;
                    }
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(src, len));
                    GlobalUnlock(mem);
                    Ok(text)
                }
            }
        };
        // SAFETY: opened above.
        unsafe { CloseClipboard() };
        result
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Candidate tools as (copy command, paste command), in order of preference.
    fn tools() -> Vec<(&'static [&'static str], &'static [&'static str])> {
        if cfg!(target_os = "macos") {
            return vec![(&["pbcopy"], &["pbpaste"])];
        }
        let mut t: Vec<(&'static [&'static str], &'static [&'static str])> = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            t.push((&["wl-copy"], &["wl-paste", "--no-newline"]));
        }
        t.push((&["xclip", "-selection", "clipboard"], &["xclip", "-selection", "clipboard", "-o"]));
        t.push((&["xsel", "--clipboard", "--input"], &["xsel", "--clipboard", "--output"]));
        t
    }

    const MISSING: &str = "no clipboard tool found (install wl-clipboard, xclip or xsel)";

    pub fn set(text: &str) -> Result<(), String> {
        for (copy, _) in tools() {
            let Ok(mut child) = Command::new(copy[0])
                .args(&copy[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            let wrote = child.stdin.take().map(|mut s| s.write_all(text.as_bytes()));
            let status = child.wait().map_err(|e| e.to_string())?;
            return match wrote {
                Some(Ok(())) if status.success() => Ok(()),
                Some(Err(e)) => Err(format!("{}: {e}", copy[0])),
                _ => Err(format!("{} failed", copy[0])),
            };
        }
        Err(MISSING.into())
    }

    pub fn get() -> Result<String, String> {
        for (_, paste) in tools() {
            let Ok(out) = Command::new(paste[0]).args(&paste[1..]).stderr(Stdio::null()).output() else {
                continue;
            };
            return if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            } else {
                // wl-paste and xclip exit non-zero when the clipboard holds no text.
                Ok(String::new())
            };
        }
        Err(MISSING.into())
    }
}
