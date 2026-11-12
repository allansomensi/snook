//! Small helpers that talk to the operating system.

/// Opens `url` in the default browser. Only `https://` addresses are opened
/// (they come from the server: never run anything else).
pub fn open_url(url: &str) -> bool {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c == '"') {
        return false;
    }
    open_default(url)
}

/// Windows: ShellExecuteW, which hands the whole address to the browser
/// without a shell. `explorer <url>` drops everything after `#`, and
/// Stripe's checkout pages need that part ("This link is incomplete").
#[cfg(windows)]
fn open_default(url: &str) -> bool {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut c_void;
    }
    let wide = |s: &str| OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (operation, file) = (wide("open"), wide(url));
    const SW_SHOWNORMAL: i32 = 1;
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call;
    // the other pointers may be null.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    r as isize > 32
}

#[cfg(not(windows))]
fn open_default(url: &str) -> bool {
    use std::process::Command;
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(program).arg(url).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_https_is_opened() {
        assert!(!super::open_url("file:///etc/passwd"));
        assert!(!super::open_url("calc.exe"));
        assert!(!super::open_url("https://x.example/a b"));
    }
}
