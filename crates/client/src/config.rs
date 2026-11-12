//! Persistent player settings, stored as a small `key = value` file in the
//! user's configuration directory.

use std::path::PathBuf;

/// Overall graphics detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    Low,
    Medium,
    High,
    Ultra,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::Low, Quality::Medium, Quality::High, Quality::Ultra];

    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "Low",
            Quality::Medium => "Medium",
            Quality::High => "High",
            Quality::Ultra => "Ultra",
        }
    }

    fn parse(s: &str) -> Option<Quality> {
        Quality::ALL.into_iter().find(|q| q.name().eq_ignore_ascii_case(s))
    }

    /// Anti-aliasing samples suggested for this preset.
    pub fn default_msaa(self) -> i32 {
        match self {
            Quality::Low => 1,
            Quality::Medium => 2,
            Quality::High => 4,
            Quality::Ultra => 8,
        }
    }
}

/// How much aiming help is drawn on the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimGuide {
    /// Line to the ghost ball, the object ball's path and the cue ball's tangent.
    Full,
    /// Line to the ghost ball only: judging the cut is up to the player.
    Standard,
    /// A short line from the cue ball (always used in rated games).
    Short,
}

impl AimGuide {
    /// The guide used against a person over the network: the standard one
    /// (ghost ball) or the short line, never the full guide (it would play
    /// the cut for you).
    pub fn online(self) -> AimGuide {
        match self {
            AimGuide::Full => AimGuide::Standard,
            g => g,
        }
    }

    pub const ALL: [AimGuide; 3] = [AimGuide::Full, AimGuide::Standard, AimGuide::Short];

    pub fn name(self) -> &'static str {
        match self {
            AimGuide::Full => "Full",
            AimGuide::Standard => "Standard",
            AimGuide::Short => "Short",
        }
    }

    fn parse(s: &str) -> Option<AimGuide> {
        AimGuide::ALL.into_iter().find(|g| g.name().eq_ignore_ascii_case(s))
    }
}

/// The default game server; change it in Settings or with `SNOOKERED_SERVER`
/// when building (an empty `SNOOKERED_SERVER`, as CI passes when the variable
/// isn't set, keeps the public server).
pub const DEFAULT_SERVER: &str = match option_env!("SNOOKERED_SERVER") {
    Some(s) if !s.is_empty() => s,
    _ => "wss://snook-server.onrender.com/ws",
};

/// What earlier builds saved as the default; read as "the default" so they
/// move to the public server (a local test server is `ws://127.0.0.1:7878/ws`).
const OLD_DEFAULT_SERVER: &str = "ws://localhost:7878/ws";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub player_name: String,
    pub second_player_name: String,
    pub guide: AimGuide,
    pub show_help: bool,
    /// Pull the camera back to watch each shot.
    pub shot_camera: bool,
    /// The camera turns with the cue (off: it stays where the player puts it).
    pub camera_follow: bool,
    /// The graphics were already matched to this computer on the first run.
    pub graphics_checked: bool,
    pub language: crate::i18n::Lang,
    /// Darkened screen edges.
    pub vignette: bool,
    /// Dark stage around the table (off: the whole bar room).
    pub focused_scene: bool,
    pub show_fps: bool,
    /// Show the round trip to the server (lounge and online matches).
    pub show_ping: bool,
    /// Interface size (1.0 = automatic for the window size).
    pub ui_size: f32,
    /// Multiplier for fine aiming with the mouse.
    pub aim_sensitivity: f32,
    pub quality: Quality,
    /// Multisample anti-aliasing samples (1 = off). Applies on restart.
    pub msaa: i32,
    pub fullscreen: bool,
    pub volume: f32,
    pub server: String,
    pub username: String,
    /// Login token for automatic sign-in (empty when logged out).
    pub token: String,
    /// Last difficulty chosen against the computer (0 easy, 1 medium, 2 hard).
    pub difficulty: u8,
    /// Offline shot clock in seconds (0: no clock).
    pub shot_clock: u32,
    /// Name used on local network games.
    pub lan_name: String,
    /// Shop items in use (as the server last confirmed them), so offline
    /// games look the same: cloth, cue, chalk and ball set.
    pub cloth: String,
    pub cue: String,
    pub chalk: String,
    pub balls: String,
    /// The winning effect on the 8.
    pub effect: String,
    /// The player declined to use their payment e-mail for recovery.
    pub email_offer_declined: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            // Empty: the field shows the default name as a hint, in the
            // current language, and that name is used if it stays empty.
            player_name: String::new(),
            second_player_name: String::new(),
            guide: AimGuide::Standard,
            show_help: false,
            shot_camera: true,
            camera_follow: false,
            graphics_checked: false,
            language: crate::i18n::Lang::system(),
            vignette: true,
            focused_scene: true,
            show_fps: false,
            show_ping: false,
            aim_sensitivity: 1.0,
            ui_size: 1.0,
            quality: Quality::High,
            msaa: 4,
            fullscreen: false,
            volume: 0.8,
            server: DEFAULT_SERVER.into(),
            username: String::new(),
            token: String::new(),
            difficulty: 1,
            shot_clock: 0,
            lan_name: String::new(),
            cloth: "cloth_green".into(),
            cue: "cue_classic".into(),
            chalk: "chalk_blue".into(),
            balls: "balls_classic".into(),
            effect: "effect_none".into(),
            email_offer_declined: false,
        }
    }
}

fn config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Snookered"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/Snookered"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|p| p.join("snookered"))
    }
}

fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.ini"))
}

impl Config {
    /// Loads the settings, falling back to defaults for anything missing.
    pub fn load() -> Config {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Config::parse(&t))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "player_name" if !v.is_empty() => c.player_name = v.chars().take(20).collect(),
                "second_player_name" if !v.is_empty() => {
                    c.second_player_name = v.chars().take(20).collect()
                }
                "guide" => c.guide = AimGuide::parse(v).unwrap_or(c.guide),
                "show_help" => c.show_help = v == "true",
                "shot_camera" => c.shot_camera = v == "true",
                "camera_follow" => c.camera_follow = v == "true",
                "graphics_checked" => c.graphics_checked = v == "true",
                "language" => c.language = crate::i18n::Lang::parse(v).unwrap_or(c.language),
                "vignette" => c.vignette = v == "true",
                "focused_scene" => c.focused_scene = v == "true",
                "show_fps" => c.show_fps = v == "true",
                "show_ping" => c.show_ping = v == "true",
                "ui_size" => {
                    c.ui_size = v.parse::<f32>().map(|x| x.clamp(0.7, 1.5)).unwrap_or(c.ui_size)
                }
                "aim_sensitivity" => {
                    c.aim_sensitivity =
                        v.parse::<f32>().map(|x| x.clamp(0.3, 2.5)).unwrap_or(c.aim_sensitivity)
                }
                "quality" => c.quality = Quality::parse(v).unwrap_or(c.quality),
                "msaa" => {
                    c.msaa =
                        v.parse::<i32>().ok().filter(|n| [1, 2, 4, 8].contains(n)).unwrap_or(c.msaa)
                }
                "fullscreen" => c.fullscreen = v == "true",
                "volume" => {
                    c.volume = v.parse::<f32>().map(|x| x.clamp(0.0, 1.0)).unwrap_or(c.volume)
                }
                "server" if !v.is_empty() && v != OLD_DEFAULT_SERVER => c.server = v.to_string(),
                "username" => c.username = v.to_string(),
                "token" => c.token = v.to_string(),
                "difficulty" => c.difficulty = v.parse::<u8>().map(|d| d.min(2)).unwrap_or(1),
                "shot_clock" => {
                    c.shot_clock =
                        v.parse::<u32>().ok().filter(|s| [0, 30, 60].contains(s)).unwrap_or(0)
                }
                "lan_name" => c.lan_name = v.chars().take(16).collect(),
                // Only real items of the right kind (a typo means the default).
                "cloth" | "cue" | "chalk" | "balls" | "effect" => {
                    let fits =
                        snookered_proto::shop::item(v).is_some_and(|i| i.category.code() == k);
                    if fits {
                        match k {
                            "cloth" => c.cloth = v.to_string(),
                            "cue" => c.cue = v.to_string(),
                            "chalk" => c.chalk = v.to_string(),
                            "effect" => c.effect = v.to_string(),
                            _ => c.balls = v.to_string(),
                        }
                    }
                }
                "email_offer_declined" => c.email_offer_declined = v == "true",
                _ => {}
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        format!(
            "# Snookered settings\nplayer_name = {}\nsecond_player_name = {}\nguide = {}\nshow_help = {}\nshot_camera = {}\ncamera_follow = {}\ngraphics_checked = {}\nlanguage = {}\nvignette = {}\nfocused_scene = {}\nshow_fps = {}\nshow_ping = {}\naim_sensitivity = {:.2}\nui_size = {:.2}\n\
             quality = {}\nmsaa = {}\nfullscreen = {}\nvolume = {:.2}\nserver = {}\nusername = {}\ntoken = {}\n\
             difficulty = {}\nshot_clock = {}\nlan_name = {}\ncloth = {}\ncue = {}\nchalk = {}\nballs = {}\neffect = {}\nemail_offer_declined = {}\n",
            self.player_name,
            self.second_player_name,
            self.guide.name(),
            self.show_help,
            self.shot_camera,
            self.camera_follow,
            self.graphics_checked,
            self.language.code(),
            self.vignette,
            self.focused_scene,
            self.show_fps,
            self.show_ping,
            self.aim_sensitivity,
            self.ui_size,
            self.quality.name(),
            self.msaa,
            self.fullscreen,
            self.volume,
            self.server,
            self.username,
            self.token,
            self.difficulty,
            self.shot_clock,
            self.lan_name,
            self.cloth,
            self.cue,
            self.chalk,
            self.balls,
            self.effect,
            self.email_offer_declined
        )
    }

    /// Writes the settings; failures are logged and otherwise ignored.
    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written to a temporary file first, so a crash never leaves a
        // half-written settings file; private to the user (it holds the
        // session token).
        let tmp = path.with_extension("ini.tmp");
        let result = write_private(&tmp, self.to_text().as_bytes())
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = result {
            eprintln!("could not save settings to {}: {e}", path.display());
        }
    }
}

/// Writes `data` to a file only the user can read, from the start (the
/// token is never readable by others, not even for a moment).
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        // A leftover file keeps its old mode: tighten it before writing.
        if let Ok(meta) = std::fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_validation() {
        let c = Config {
            player_name: "Allan".into(),
            quality: Quality::Low,
            msaa: 8,
            volume: 0.25,
            token: "abc".into(),
            ..Config::default()
        };
        assert_eq!(Config::parse(&c.to_text()), c);
        let bad = Config::parse("msaa = 3\nvolume = 9\nquality = potato\nnonsense");
        assert_eq!(bad.msaa, Config::default().msaa);
        assert_eq!(bad.volume, 1.0);
        assert_eq!(bad.quality, Quality::High);
    }
}
