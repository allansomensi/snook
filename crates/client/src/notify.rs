//! Telling a player who is in another window: a notification from the
//! system (and, on Windows, a flashing taskbar button) when an invitation
//! arrives, "Play now" finds an opponent or a friend comes online.
//!
//! The game can't see whether its window has the focus, so it notifies when
//! nobody has touched it for a while (the player is somewhere else).

use std::time::{Duration, Instant};

use macroquad::prelude::*;

/// Nobody touched the game for this long: they are probably elsewhere.
const AWAY_AFTER: Duration = Duration::from_secs(12);
/// At most one notification this often.
const MIN_GAP: Duration = Duration::from_secs(8);

pub struct Notifier {
    last_input: Instant,
    last_mouse: Vec2,
    last_sent: Option<Instant>,
}

impl Default for Notifier {
    fn default() -> Self {
        Notifier { last_input: Instant::now(), last_mouse: Vec2::ZERO, last_sent: None }
    }
}

impl Notifier {
    /// Call every frame: notices the player using the game.
    pub fn update(&mut self) {
        let mouse = Vec2::from(mouse_position());
        let used = mouse != self.last_mouse
            || get_last_key_pressed().is_some()
            || is_mouse_button_down(MouseButton::Left)
            || mouse_wheel().1 != 0.0;
        self.last_mouse = mouse;
        if used {
            self.last_input = Instant::now();
        }
    }

    /// Shows `text` from the system if the player seems to be away.
    pub fn tell(&mut self, text: &str) {
        if self.last_input.elapsed() < AWAY_AFTER {
            return;
        }
        if self.last_sent.is_some_and(|t| t.elapsed() < MIN_GAP) {
            return;
        }
        self.last_sent = Some(Instant::now());
        let text = text.to_string();
        // Never hold the game up: the commands run on their own.
        std::thread::spawn(move || send(&text));
    }
}

fn send(text: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .args([
                &format!("--app-name={}", snook_proto::GAME_NAME),
                "--urgency=normal",
                snook_proto::GAME_NAME,
                text,
            ])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let quoted = text.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{quoted}\" with title \"{}\" sound name \"Glass\"",
            snook_proto::GAME_NAME
        );
        let _ = std::process::Command::new("osascript").args(["-e", &script]).spawn();
    }
    #[cfg(windows)]
    {
        windows::flash();
        windows::toast(text);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let _ = text;
}

#[cfg(windows)]
mod windows {
    use std::os::windows::process::CommandExt;

    #[repr(C)]
    struct FlashWInfo {
        cb_size: u32,
        hwnd: *mut core::ffi::c_void,
        flags: u32,
        count: u32,
        timeout: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut core::ffi::c_void;
        fn FlashWindowEx(info: *const FlashWInfo) -> i32;
    }

    /// Flashes the game's taskbar button until it is brought to the front.
    pub fn flash() {
        let title: Vec<u16> = snook_proto::GAME_NAME.encode_utf16().chain(Some(0)).collect();
        // SAFETY: a null-terminated UTF-16 title and a null class are valid
        // arguments; the window handle is only passed back to the system.
        unsafe {
            let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
            if hwnd.is_null() {
                return;
            }
            const FLASHW_ALL: u32 = 3;
            const FLASHW_TIMERNOFG: u32 = 12;
            let info = FlashWInfo {
                cb_size: std::mem::size_of::<FlashWInfo>() as u32,
                hwnd,
                flags: FLASHW_ALL | FLASHW_TIMERNOFG,
                count: 0,
                timeout: 0,
            };
            FlashWindowEx(&info);
        }
    }

    /// A toast from the system (Windows 10 and later).
    pub fn toast(text: &str) {
        let quoted = text.replace('\'', "''");
        let script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
             $t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $n = $t.GetElementsByTagName('text'); \
             $n.Item(0).AppendChild($t.CreateTextNode('{name}')) > $null; \
             $n.Item(1).AppendChild($t.CreateTextNode('{quoted}')) > $null; \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{name}').Show([Windows.UI.Notifications.ToastNotification]::new($t))",
            name = snook_proto::GAME_NAME
        );
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
}
