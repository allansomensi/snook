//! Getting the new version when the server says this one is out of date:
//! the Microsoft Store's updates page for the Store version; on Windows, the
//! installed game downloads the new installer and runs it; anywhere else,
//! the releases page.

/// The latest release's page.
pub const RELEASES: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/latest");

/// The Windows installer of the latest release.
#[cfg_attr(not(windows), allow(dead_code))]
const MSI: &str = concat!(
    env!("CARGO_PKG_REPOSITORY"),
    "/releases/latest/download/snookered-x86_64-pc-windows-msvc.msi"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// The Microsoft Store updates the game.
    Store,
    /// Installed with the .msi: download the new one and run it.
    Installer,
    /// A portable copy, macOS or Linux: the releases page.
    Page,
}

pub fn how() -> How {
    if crate::channel::STORE {
        return How::Store;
    }
    if cfg!(windows) && installed_with_msi() {
        return How::Installer;
    }
    How::Page
}

/// The .msi installs under Program Files.
fn installed_with_msi() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_ascii_lowercase()))
        .is_some_and(|p| p.contains("\\program files"))
}

/// Starts getting the new version. Returns true when the game should close
/// (the installer replaces it).
pub fn start() -> bool {
    match how() {
        How::Store => {
            crate::util::open_store_updates();
            false
        }
        How::Page => {
            crate::util::open_url(RELEASES);
            false
        }
        How::Installer => run_installer(),
    }
}

#[cfg(windows)]
fn run_installer() -> bool {
    use std::os::windows::process::CommandExt;
    // Downloads the installer, waits for the game to close, then runs it;
    // if anything fails, the releases page opens instead.
    let script = format!(
        "$p = Join-Path $env:TEMP 'snookered-update.msi'; \
         try {{ \
           Invoke-WebRequest -UseBasicParsing -Uri '{MSI}' -OutFile $p; \
           Wait-Process -Id {pid} -Timeout 60 -ErrorAction SilentlyContinue; \
           Start-Process msiexec -ArgumentList @('/i', ('\"' + $p + '\"')) \
         }} catch {{ Start-Process '{RELEASES}' }}",
        pid = std::process::id()
    );
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .is_ok()
}

#[cfg(not(windows))]
fn run_installer() -> bool {
    crate::util::open_url(RELEASES);
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn links_point_at_the_latest_release() {
        assert!(super::RELEASES.starts_with("https://github.com/"));
        assert!(super::RELEASES.ends_with("/releases/latest"));
        assert!(super::MSI.ends_with(".msi") && super::MSI.contains("/releases/latest/download/"));
        assert_eq!(super::how(), super::How::Page, "not under Program Files here");
    }
}
