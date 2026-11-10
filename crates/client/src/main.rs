// No console window behind the game on Windows (release builds).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Snook desktop client.
//!
//! Rendering uses macroquad (OpenGL 2.1 / GLES 2), so the game is small and
//! runs on practically any PC, including old integrated graphics.

mod audio;
mod camera;
mod config;
mod coords;
mod game;
mod gfx;
mod hud;
mod i18n;
mod icon;
mod menu;
mod net;
mod online;
mod play;
mod scene;
mod settings;
mod textures;
mod ui;

use macroquad::prelude::*;
use snook_core::{Difficulty, Table};

use crate::camera::{CameraMode, CameraRig};
use crate::config::Config;
use crate::game::Match;
use crate::hud::{Hud, HudAction};
use crate::menu::{Menu, MenuAction};
use crate::online::{Online, OnlineAction};
use crate::play::Controls;
use crate::scene::{Overlay, Scene};
use crate::settings::{SettingsAction, SettingsScreen};
use crate::ui::Ui;

/// Who plays a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    VsComputer(Difficulty),
    HotSeat,
    /// Computer against computer (attract mode and testing).
    Demo,
}

/// Choices made in the main menu.
#[derive(Debug, Clone)]
pub struct MatchSettings {
    pub mode: MatchMode,
    pub player_name: String,
    pub second_player_name: String,
    pub guide: config::AimGuide,
}

impl MatchSettings {
    fn from_config(mode: MatchMode, c: &Config) -> Self {
        MatchSettings {
            mode,
            // An empty name field means the default name (the field itself is
            // left empty while the player types).
            player_name: name_or(&c.player_name, "Player"),
            second_player_name: name_or(&c.second_player_name, "Player 2"),
            guide: c.guide,
        }
    }
}

/// Command-line options (mostly for development and testing).
#[derive(Default)]
struct LaunchOptions {
    start: Option<MatchMode>,
    top_view: bool,
    screenshot: Option<String>,
    frames: u32,
    show_fps: bool,
    online: bool,
    settings: bool,
    on_eight: bool,
}

fn parse_args() -> LaunchOptions {
    let mut o = LaunchOptions { frames: 120, ..Default::default() };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => o.start = Some(MatchMode::Demo),
            "--hotseat" => o.start = Some(MatchMode::HotSeat),
            "--vs" => {
                let level = match args.next().as_deref() {
                    Some("easy") => Difficulty::Easy,
                    Some("hard") => Difficulty::Hard,
                    _ => Difficulty::Medium,
                };
                o.start = Some(MatchMode::VsComputer(level));
            }
            "--top-view" => o.top_view = true,
            "--fps" => o.show_fps = true,
            "--online" => o.online = true,
            "--settings" => o.settings = true,
            "--on-eight" => o.on_eight = true,
            "--screenshot" => o.screenshot = args.next(),
            "--frames" => o.frames = args.next().and_then(|s| s.parse().ok()).unwrap_or(120),
            "--help" | "-h" => {
                println!(
                    "Snook\n\n  --vs <easy|medium|hard>  play the computer\n  --hotseat                two players\n  \
                     --demo                   computer vs computer\n  --top-view               overhead camera\n  \
                     --online                 open the online lounge\n  \
                     --settings               open the settings\n  \
                     --screenshot <file.png>  save a screenshot after --frames <n> frames and quit"
                );
                std::process::exit(0);
            }
            other => eprintln!("ignoring unknown argument `{other}`"),
        }
    }
    o
}

fn window_conf() -> macroquad::conf::Conf {
    let config = Config::load();
    let window = Conf {
        window_title: "Snook".into(),
        window_width: 1280,
        window_height: 760,
        // Safe graphics mode (troubleshooting) also turns multisampling off.
        sample_count: if std::env::var_os("SNOOK_SAFE_GRAPHICS").is_some() {
            1
        } else {
            config.msaa
        },
        fullscreen: config.fullscreen,
        window_resizable: true,
        high_dpi: false,
        icon: Some(icon::window_icon()),
        platform: macroquad::miniquad::conf::Platform {
            // Vertical sync: no tearing, and no burning the GPU/battery for
            // frames the screen can't show.
            swap_interval: Some(1),
            ..Default::default()
        },
    };
    macroquad::conf::Conf {
        miniquad_conf: window,
        update_on: None,
        default_filter_mode: FilterMode::Linear,
        // macroquad's defaults: large per-draw-call buffers cost a lot of
        // video memory and upset some drivers (big meshes are split instead).
        draw_call_vertex_capacity: 10_000,
        draw_call_index_capacity: 5_000,
    }
}

/// Which full-screen page is open over the 3D table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Online,
}

#[macroquad::main(window_conf)]
async fn main() {
    let options = parse_args();
    let mut ui = Ui::new();
    // Show something while the textures are generated.
    clear_background(Color::new(0.05, 0.035, 0.025, 1.0));
    ui.text_centered(
        "Snook",
        screen_width() / 2.0,
        screen_height() / 2.0 - 20.0,
        32.0,
        ui::GOLD,
        true,
    );
    next_frame().await;

    let mut config = Config::load();
    i18n::set_lang(config.language);
    let mut scene = Scene::new(Table::default(), config.quality, config.focused_scene);
    let mut retired: Vec<Scene> = Vec::new();
    let sounds = audio::Sounds::load().await;
    let mut menu = Menu::default();
    let mut hud = Hud::new(config.show_help);
    let mut rig = CameraRig::default();
    rig.follow_cue = config.camera_follow;
    rig.reset();
    let mut controls = Controls::default();
    let mut current: Option<Match> = None;
    let mut screen = Screen::Menu;
    let mut online: Option<Online> = None;
    let mut settings_open = options.settings;
    let mut settings_screen = SettingsScreen::default();
    if options.top_view {
        rig.mode = CameraMode::Top;
    }
    if let Some(mode) = options.start {
        let mut m = Match::new(&MatchSettings::from_config(mode, &config), game::clock_seed());
        if options.on_eight {
            m.debug_on_eight();
        }
        current = Some(m);
    }
    if options.online {
        online = Some(Online::start(&config));
        screen = Screen::Online;
    }
    let menu_rack = snook_core::rules::rack(&scene.table, 2024);
    let vignette = vignette_texture();

    // First run: measure the frame rate on the menu and pick graphics that
    // stay smooth on this computer.
    let mut probe = (0.0f32, 0u32);
    let mut toast: Option<(String, f32)> = None;
    // Fades the screen in from black after a scene change.
    let mut fade = 1.0f32;
    let mut was_in_match = current.is_some();
    let mut frame = 0u32;
    loop {
        let dt = get_frame_time().min(0.1);
        ui.begin();

        if is_key_pressed(KeyCode::F11) {
            config.fullscreen = !config.fullscreen;
            set_fullscreen(config.fullscreen);
            config.save();
        }

        // Network traffic (lounge and online matches).
        if let Some(o) = online.as_mut() {
            let was_in_match = current.as_ref().is_some_and(|m| m.is_online());
            if o.update(dt, &mut current, &mut config) {
                config.save();
            }
            let in_match = current.as_ref().is_some_and(|m| m.is_online());
            if in_match && !was_in_match {
                hud = Hud::new(config.show_help);
                rig.reset();
                settings_open = false;
            } else if was_in_match && !in_match {
                hud = Hud::new(config.show_help);
                rig.reset();
            }
        }

        if let Some(m) = current.as_mut() {
            if !hud.paused || m.is_online() {
                m.update(dt);
            }
            sounds.play(&mut m.sounds, config.volume);
        }

        rig.follow_shots = config.shot_camera;
        if rig.follow_cue != config.camera_follow {
            rig.follow_cue = config.camera_follow;
            rig.reset();
        }
        hud.top_view = rig.mode == CameraMode::Top;
        hud.aim_sensitivity = config.aim_sensitivity;
        controls.sensitivity = config.aim_sensitivity;
        let mut cam = rig.update(dt, current.as_ref(), get_time() as f32);
        // Development aid: SNOOK_CAM="eye x,y,z,target x,y,z" fixes the camera.
        if let Some(v) = std::env::var("SNOOK_CAM")
            .ok()
            .map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>())
            && v.len() == 6
        {
            cam.eye = vec3(v[0], v[1], v[2]);
            cam.target = vec3(v[3], v[4], v[5]);
            cam.up = Vec3::Y;
        }
        if let Some(m) = current.as_mut()
            && !hud.paused
            && !settings_open
        {
            controls.camera(&mut rig, &cam, ui.hovered, ui.wants_keyboard);
            controls.player(m, &scene, &cam, &mut rig, ui.hovered, ui.wants_keyboard);
        }

        clear_background(if config.focused_scene {
            Color::new(0.055, 0.05, 0.048, 1.0)
        } else {
            Color::new(0.03, 0.02, 0.015, 1.0)
        });
        scene.lamp_visible = !(current.is_some() && rig.mode == CameraMode::Top);
        match current.as_ref() {
            Some(m) => {
                let overlay = play::overlay(m, &scene, get_time() as f32);
                scene.draw(&cam, m.balls(), dt, &overlay);
            }
            None => scene.draw(&cam, &menu_rack, dt, &Overlay::default()),
        }

        if config.vignette {
            draw_texture_ex(
                &vignette,
                0.0,
                0.0,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(screen_width(), screen_height())),
                    ..Default::default()
                },
            );
        }

        // 2D interface.
        if settings_open {
            let before = config.clone();
            let action = settings_screen.draw(&mut ui, &mut config, current.is_some());
            if config.quality != before.quality || config.focused_scene != before.focused_scene {
                // The old scene's materials may still be referenced by draw calls
                // queued this frame: drop it after the frame is shown.
                retired.push(std::mem::replace(
                    &mut scene,
                    Scene::new(Table::default(), config.quality, config.focused_scene),
                ));
            }
            if config.language != before.language {
                i18n::set_lang(config.language);
            }
            if config.show_help != before.show_help {
                hud.show_help = config.show_help;
            }
            if let Some(m) = current.as_mut() {
                let rated = m.online.as_ref().is_some_and(|o| o.rated);
                m.guide = if rated { config::AimGuide::Short } else { config.guide };
            }
            if let SettingsAction::Close = action {
                settings_open = false;
                config.save();
            }
        } else {
            match current.as_mut() {
                Some(m) => match hud.draw(&mut ui, m) {
                    HudAction::MainMenu => {
                        current = None;
                        hud = Hud::new(config.show_help);
                        rig.reset();
                    }
                    HudAction::Lounge => {
                        if let Some(o) = online.as_mut() {
                            o.leave_match(m);
                        }
                        current = None;
                        hud = Hud::new(config.show_help);
                        rig.reset();
                    }
                    HudAction::Settings => settings_open = true,
                    HudAction::ToggleView => {
                        if rig.mode == CameraMode::Top {
                            rig.enter_3d();
                        } else {
                            rig.reset();
                        }
                    }
                    HudAction::ResetCamera => rig.reset(),
                    HudAction::None => {
                        if hud.show_help != config.show_help {
                            config.show_help = hud.show_help;
                            config.save();
                        }
                        if m.is_online()
                            && let Some(o) = online.as_mut()
                            && o.pending_invites() > 0
                        {
                            o.draw_invites(&mut ui, true);
                        }
                    }
                },
                None => match screen {
                    Screen::Menu => match menu.draw(&mut ui, &mut config) {
                        MenuAction::Start(mode) => {
                            config.save();
                            let settings = MatchSettings::from_config(mode, &config);
                            current = Some(Match::new(&settings, game::clock_seed()));
                            hud = Hud::new(config.show_help);
                            rig.reset();
                        }
                        MenuAction::Online => {
                            config.save();
                            online = Some(Online::start(&config));
                            screen = Screen::Online;
                        }
                        MenuAction::Settings => settings_open = true,
                        MenuAction::Quit => break,
                        MenuAction::None => {}
                    },
                    Screen::Online => {
                        let action = match online.as_mut() {
                            Some(o) => o.draw(&mut ui, &mut config),
                            None => OnlineAction::Leave,
                        };
                        if let OnlineAction::Leave = action {
                            online = None;
                            screen = Screen::Menu;
                        }
                    }
                },
            }
        }

        if !config.graphics_checked
            && current.is_none()
            && !settings_open
            && options.screenshot.is_none()
        {
            probe.0 += get_frame_time();
            probe.1 += 1;
            if probe.0 > 5.0 {
                // Average over the first seconds on the menu.
                let fps = probe.1 as f32 / probe.0;
                let pick = if fps < 22.0 {
                    Some(config::Quality::Low)
                } else if fps < 40.0 && config.quality > config::Quality::Medium {
                    Some(config::Quality::Medium)
                } else {
                    None
                };
                if let Some(q) = pick {
                    config.quality = q;
                    config.msaa = config.msaa.min(q.default_msaa());
                    retired.push(std::mem::replace(
                        &mut scene,
                        Scene::new(Table::default(), q, config.focused_scene),
                    ));
                    toast = Some((
                        i18n::tw(
                            "Graphics set to {q} for smooth play. You can change this in Settings.",
                            &[("q", &i18n::t(q.name()))],
                        ),
                        7.0,
                    ));
                }
                config.graphics_checked = true;
                config.save();
            }
        }
        if let Some((text, t)) = &mut toast {
            *t -= dt;
            let alpha = t.clamp(0.0, 1.0);
            let w = ui.text_size(text, 15.0, false).x + 32.0;
            let r = Rect::new((screen_width() - w) / 2.0, screen_height() - 60.0, w, 34.0);
            ui::rrect(r, 17.0, Color::new(0.0, 0.0, 0.0, 0.7 * alpha));
            ui.text_centered(
                text,
                screen_width() / 2.0,
                r.y + 9.0,
                15.0,
                Color::new(1.0, 0.95, 0.85, alpha),
                false,
            );
            if *t <= 0.0 {
                toast = None;
            }
        }

        let in_match = current.is_some();
        if in_match != was_in_match {
            was_in_match = in_match;
            fade = 0.85;
        }
        if fade > 0.0 {
            draw_rectangle(
                0.0,
                0.0,
                screen_width(),
                screen_height(),
                Color::new(0.0, 0.0, 0.0, fade),
            );
            fade -= dt * 2.2;
        }

        if config.show_fps || options.show_fps {
            let text = format!("{} FPS", get_fps());
            let w = ui.text_size(&text, 13.0, true).x;
            ui.text(
                &text,
                screen_width() - w - 10.0,
                screen_height() - 20.0,
                13.0,
                ui::MUTED,
                true,
            );
        }
        frame += 1;
        if options.show_fps && frame.is_multiple_of(120) {
            println!("fps {}", get_fps());
        }
        if let Some(path) = &options.screenshot
            && frame == options.frames
        {
            let mut image = get_screen_data();
            // The framebuffer's alpha is meaningless; save an opaque picture.
            for px in image.bytes.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
            image.export_png(path);
            break;
        }
        next_frame().await;
        retired.clear();
    }
    config.save();
}

/// Darkened screen corners, like a camera lens: draws the eye to the table.
fn vignette_texture() -> Texture2D {
    const N: u16 = 128;
    let mut rgba = Vec::with_capacity(N as usize * N as usize * 4);
    for y in 0..N {
        for x in 0..N {
            let u = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let d = (u * u * 0.8 + v * v).sqrt();
            let a = ((d - 0.55) / 0.85).clamp(0.0, 1.0);
            let a = a * a * (3.0 - 2.0 * a) * 0.55;
            rgba.extend_from_slice(&[0, 0, 0, (a * 255.0) as u8]);
        }
    }
    let tex = Texture2D::from_rgba8(N, N, &rgba);
    tex.set_filter(FilterMode::Linear);
    tex
}

fn name_or(name: &str, default: &str) -> String {
    match name.trim() {
        "" => i18n::t(default).to_string(),
        n => n.to_string(),
    }
}
