//! Small helpers that talk to the operating system.

/// Opens `url` in the default browser. Only `https://` addresses are opened
/// (they come from the server: never run anything else).
pub fn open_url(url: &str) -> bool {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c == '"') {
        return false;
    }
    use std::process::Command;
    let result = if cfg!(target_os = "windows") {
        // `explorer` opens the default browser without going through a shell.
        Command::new("explorer").arg(url).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    result.is_ok()
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
