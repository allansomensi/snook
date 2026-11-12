// No console window behind the game on Windows (release builds).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Snook desktop client.
//!
//! Rendering uses macroquad (OpenGL 2.1 / GLES 2), so the game is small and
//! runs on practically any PC, including old integrated graphics.

mod audio;
mod camera;
mod channel;
mod config;
mod coords;
mod countries;
mod dev;
mod effects;
mod flags;
mod game;
mod gfx;
mod hud;
mod i18n;
mod icon;
mod icons;
mod lan;
mod menu;
mod net;
mod notify;
mod online;
mod paint;
mod play;
mod profile;
mod ranking;
mod reactions;
mod room;
mod scene;
mod settings;
mod setup;
mod shop;
mod textures;
mod ui;
mod util;

use macroquad::prelude::*;
use snook_core::{Difficulty, Table};

use crate::camera::{CameraMode, CameraRig};
use crate::config::Config;
use crate::game::Match;
use crate::hud::{Hud, HudAction};
use crate::lan::{LanAction, LanScreen};
use crate::menu::{Menu, MenuAction};
use crate::online::{Online, OnlineAction};
use crate::play::Controls;
use crate::profile::{ProfileAction, ProfileScreen};
use crate::ranking::{Ranking, RankingAction};
use crate::scene::{Overlay, Scene};
use crate::settings::{SettingsAction, SettingsScreen};
use crate::setup::{Setup, SetupAction, SetupKind};
use crate::ui::Ui;

/// Who plays a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    VsComputer(Difficulty),
    HotSeat,
    /// Computer against computer (attract mode and testing).
    Demo,
}

/// Choices made before an offline match.
#[derive(Debug, Clone)]
pub struct MatchSettings {
    pub mode: MatchMode,
    pub names: [String; 2],
    pub guide: config::AimGuide,
    /// Seconds per shot, if timed.
    pub shot_clock: Option<u32>,
}

impl MatchSettings {
    fn from_config(mode: MatchMode, c: &Config) -> Self {
        MatchSettings {
            mode,
            // An empty name field means the default name (the field itself is
            // left empty while the player types).
            names: [name_or(&c.player_name, "Player"), name_or(&c.second_player_name, "Player 2")],
            guide: c.guide,
            shot_clock: (c.shot_clock > 0).then_some(c.shot_clock),
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
    screen: Option<String>,
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
            "--screen" => o.screen = args.next(),
            "--window" => {
                args.next();
            }
            "--screenshot" => o.screenshot = args.next(),
            "--frames" => o.frames = args.next().and_then(|s| s.parse().ok()).unwrap_or(120),
            "--help" | "-h" => {
                println!(
                    "Snookered\n\n  --vs <easy|medium|hard>  play the computer\n  --hotseat                two players\n  \
                     --demo                   computer vs computer\n  --top-view               overhead camera\n  \
                     --online                 open the online lounge\n  \
                     --settings               open the settings\n  \
                     --screen <name>          open a screen (lan, setup, ranking)\n  \
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
    // `--window WxH` (development): start with this window size.
    let args: Vec<String> = std::env::args().collect();
    let size = args
        .iter()
        .position(|a| a == "--window")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.split_once('x'))
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .unwrap_or((1280, 760));
    let window = Conf {
        window_title: snook_proto::GAME_NAME.into(),
        window_width: size.0,
        window_height: size.1,
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
enum Screen {
    Menu,
    Setup(Setup),
    /// The online lounge (or the sign-in form).
    Online,
    Lan,
    /// The lounge of a LAN game.
    LanLounge,
    Ranking,
    Profile,
    Shop,
}

/// A LAN game: the server this computer hosts (if any) and the connection.
struct LanSession {
    host: Option<snook_server::lan::LanHost>,
    net: Online,
}

#[macroquad::main(window_conf)]
async fn main() {
    let options = parse_args();
    let mut ui = Ui::new();
    // Show something while the textures are generated.
    clear_background(Color::new(0.05, 0.035, 0.025, 1.0));
    {
        let size = (screen_height() * 0.22).clamp(96.0, 220.0);
        icon::draw((screen_width() - size) / 2.0, screen_height() / 2.0 - size * 0.75, size);
        ui.text_centered(
            &snook_proto::GAME_NAME.to_uppercase(),
            screen_width() / 2.0,
            screen_height() / 2.0 + size * 0.35,
            32.0,
            ui::GOLD,
            true,
        );
    }
    next_frame().await;

    let mut config = Config::load();
    i18n::set_lang(config.language);
    let mut scene = Scene::new(
        Table::default(),
        config.quality,
        config.focused_scene,
        &scene::Looks::of(&config.cloth, &config.balls),
    );
    let mut retired: Vec<Scene> = Vec::new();
    let sounds = audio::Sounds::load().await;
    let mut menu = Menu::default();
    let mut hud = Hud::new(config.show_help);
    let mut rig = CameraRig::default();
    rig.follow_cue = config.camera_follow;
    rig.reset();
    let mut controls = Controls::default();
    let mut current: Option<Match> = None;
    // The online match came from the LAN game (not the global server).
    let mut match_from_lan = false;
    let mut screen = Screen::Menu;
    // The global server: signs in in the background when a session was saved.
    let mut online = Online::new(&config);
    let mut lan_screen: Option<LanScreen> = None;
    let mut lan: Option<LanSession> = None;
    let mut ranking = Ranking::default();
    let mut profile = ProfileScreen::default();
    let mut shop = shop::ShopScreen::default();
    // The shop item being tried on the 3D table, and since when.
    let mut shop_preview: Option<(&'static str, f32)> = None;
    // Winning effects playing over the table.
    let mut win_effects: Vec<effects::WinEffect> = Vec::new();
    // System notifications for a player in another window.
    let mut notifier = notify::Notifier::default();
    // Where the profile and ranking go back to.
    let mut back_to_lounge = false;
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
        // Development aid: SNOOK_RESIGN=1 resigns at once (the game-over panel).
        if crate::dev::flag("SNOOK_RESIGN") {
            m.resign();
        }
        current = Some(m);
    }
    if options.online {
        online.connect();
        screen = Screen::Online;
    }
    match options.screen.as_deref() {
        Some("lan") => {
            lan_screen = Some(LanScreen::new());
            screen = Screen::Lan;
        }
        Some("setup") => screen = Screen::Setup(Setup::new(SetupKind::VsComputer)),
        Some(tab) if tab.starts_with("shop") => {
            online.connect();
            shop.open(&mut online);
            shop.dev_tab(tab);
            screen = Screen::Shop;
        }
        Some("ranking") => {
            ranking.open(&mut online);
            screen = Screen::Ranking;
        }
        Some(tab @ ("tables" | "tournaments" | "friends")) => {
            online.connect();
            online.tab = match tab {
                "tables" => online::Tab::Tables,
                "tournaments" => online::Tab::Tournaments,
                _ => online::Tab::Friends,
            };
            screen = Screen::Online;
        }
        Some(name) if name.starts_with("profile:") => {
            profile.open(&name["profile:".len()..], &mut online);
            screen = Screen::Profile;
        }
        _ => {}
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
    let mut hud_opacity = 1.0f32;
    let mut frame = 0u32;
    // Development aid: SNOOK_PROFILE prints the CPU time of a frame (without
    // waiting for the GPU) every 300 frames.
    let profiling = crate::dev::flag("SNOOK_PROFILE");
    let mut cpu = (0.0f64, 0.0f64, 0u32);
    loop {
        let frame_start = std::time::Instant::now();
        let dt = get_frame_time().min(0.1);
        ui::update_scale(config.ui_size);
        ui.begin();

        if is_key_pressed(KeyCode::F11) {
            config.fullscreen = !config.fullscreen;
            set_fullscreen(config.fullscreen);
            config.save();
        }

        // Network traffic. Only the server that owns the current online match
        // gets it (and sends its messages).
        let online_match = current.as_ref().is_some_and(|m| m.is_online());
        let was_online_match = online_match;
        {
            let mut none = None;
            let global_slot = if online_match && match_from_lan { &mut none } else { &mut current };
            if online.update(dt, global_slot, &mut config) {
                config.save();
            }
            if !was_online_match && current.as_ref().is_some_and(|m| m.is_online()) {
                match_from_lan = false;
            }
        }
        if let Some(l) = lan.as_mut() {
            let mut none = None;
            let owned = current.as_ref().is_some_and(|m| m.is_online());
            let slot = if owned && !match_from_lan { &mut none } else { &mut current };
            let before = slot.as_ref().is_some_and(|m| m.is_online());
            l.net.update(dt, slot, &mut config);
            if !before && current.as_ref().is_some_and(|m| m.is_online()) {
                match_from_lan = true;
            }
        }
        notifier.update();
        for net in [Some(&mut online), lan.as_mut().map(|l| &mut l.net)].into_iter().flatten() {
            if let Some(text) = net.attention.take() {
                notifier.tell(&text);
            }
        }
        // A stored game to watch again (from the profile).
        if let Some(replay) = online.replay.take()
            && !current.as_ref().is_some_and(|m| m.is_online())
        {
            current = Some(Match::replay_of(&replay, config.guide));
            hud = Hud::new(config.show_help);
            rig.reset();
        }
        let in_online_match = current.as_ref().is_some_and(|m| m.is_online());
        if in_online_match != was_online_match {
            hud = Hud::new(config.show_help);
            rig.reset();
            if in_online_match {
                settings_open = false;
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
        hud.camera_default = rig.is_default();
        hud.aim_sensitivity = config.aim_sensitivity;
        hud.zoom = rig.zoom_level();
        controls.sensitivity = config.aim_sensitivity;
        // New cloth or balls from the shop: dress the table again (in the
        // background). Online, the table is the host's (or the higher
        // rated player's).
        if std::mem::take(&mut online.looks_changed) {
            config.save();
        }
        let table_look = current
            .as_ref()
            .filter(|m| m.is_online() && !match_from_lan)
            .map(|_| &online.table_look)
            .filter(|l| !l[0].is_empty());
        let mut looks = match table_look {
            Some(l) => scene::Looks::of(&l[0], &l[1]),
            None => scene::Looks::of(&config.cloth, &config.balls),
        };
        // Trying an item from the shop on the table.
        let previewing =
            shop_preview.filter(|_| matches!(screen, Screen::Shop) && current.is_none());
        if let Some((code, _)) = previewing {
            let cloth = if code.starts_with("cloth") { code } else { config.cloth.as_str() };
            let balls = if code.starts_with("balls") { code } else { config.balls.as_str() };
            looks = scene::Looks::of(cloth, balls);
        }
        scene.set_looks(&looks);
        // Offline, the people at the table play with this player's cue
        // (and win with their effect).
        if let Some(m) = current.as_mut().filter(|m| !m.is_online()) {
            for seat in 0..2 {
                if m.controllers[seat] == game::Controller::Human {
                    let look = (config.cue.clone(), config.chalk.clone());
                    if m.cue_looks[seat] != look {
                        m.cue_looks[seat] = look;
                    }
                    if m.effects[seat] != config.effect {
                        m.effects[seat] = config.effect.clone();
                    }
                }
            }
        }
        // A game won on the 8: the winner's effect over the pocket.
        if let Some(m) = current.as_mut()
            && let Some((seat, pocket)) = m.win_effect.take()
        {
            let at = coords::surface(scene.pocket_center(pocket), 0.04);
            win_effects.extend(effects::WinEffect::of(&m.effects[seat], at));
        }
        let mut cam = rig.update(dt, current.as_ref(), get_time() as f32);
        // The shop's 3D preview: the camera turns slowly around the table.
        if let Some((_, since)) = previewing {
            let a = (get_time() as f32 - since) * 0.22 + 0.6;
            let close =
                shop_preview.is_some_and(|(c, _)| c.starts_with("cue") || c.starts_with("chalk"));
            let (radius, height) = if close { (1.35, 1.25) } else { (2.9, 1.75) };
            cam.eye = vec3(a.cos() * radius * 1.15, height, a.sin() * radius * 0.8);
            cam.target = if close {
                vec3(-0.55, coords::TABLE_HEIGHT + 0.05, 0.0)
            } else {
                vec3(0.0, coords::TABLE_HEIGHT - 0.1, 0.0)
            };
            cam.up = Vec3::Y;
        }
        // Development aid: SNOOK_CAM="eye x,y,z,target x,y,z" fixes the camera.
        if let Some(v) = crate::dev::var("SNOOK_CAM")
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
            // The lounge fills the window and hides the table: don't spend
            // the GPU (and the battery) drawing it behind.
            None if matches!(screen, Screen::LanLounge)
                || (matches!(screen, Screen::Online) && online.signed_in()) => {}
            None => {
                let mut overlay = Overlay::default();
                if let Some((code, _)) =
                    previewing.filter(|(c, _)| c.starts_with("cue") || c.starts_with("chalk"))
                {
                    // The cue lined up behind the cue ball.
                    let cue = if code.starts_with("cue") { code } else { config.cue.as_str() };
                    let chalk =
                        if code.starts_with("chalk") { code } else { config.chalk.as_str() };
                    if let Some(ball) = menu_rack.iter().find(|b| b.id == snook_core::BallId::CUE) {
                        let params = snook_core::PhysicsParams::default();
                        let pose = scene.cue_pose(
                            &params,
                            ball.position,
                            0.0,
                            0.08,
                            ::glam::DVec2::ZERO,
                            0.03,
                        );
                        overlay.cue = Some(pose);
                        overlay.cue_look = (cue.to_string(), chalk.to_string());
                    }
                }
                scene.draw(&cam, &menu_rack, dt, &overlay)
            }
        }

        // From here on: the 2D interface, in interface units.
        ui::begin_ui_camera();
        if !win_effects.is_empty() {
            let k = ui::ui_scale();
            let scale = ui::vh() * 0.22;
            win_effects.retain_mut(|e| e.draw(dt, cam.project(e.at).map(|p| p / k), scale));
        }
        if config.vignette {
            draw_texture_ex(
                &vignette,
                0.0,
                0.0,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(ui::vw(), ui::vh())),
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
                    Scene::new(
                        Table::default(),
                        config.quality,
                        config.focused_scene,
                        &scene::Looks::of(&config.cloth, &config.balls),
                    ),
                ));
            }
            if config.language != before.language {
                i18n::set_lang(config.language);
            }
            if config.server != before.server && !online.signed_in() {
                online = Online::new(&config);
            }
            if let Some(m) = current.as_mut() {
                m.guide = match m.online.as_ref() {
                    Some(_) => config.guide.online(),
                    None => config.guide,
                };
            }
            if let SettingsAction::Close = action {
                settings_open = false;
                config.save();
            }
        } else {
            match current.as_mut() {
                Some(m) => {
                    let action =
                        hud_draw(&mut hud, &mut ui, m, controls.aiming(), &mut hud_opacity, dt);
                    match action {
                        HudAction::MainMenu => {
                            // Back from a replay: to the profile it was opened from.
                            let replay = m.is_replay();
                            current = None;
                            hud = Hud::new(config.show_help);
                            rig.reset();
                            screen = if replay { Screen::Profile } else { Screen::Menu };
                        }
                        HudAction::Lounge => {
                            if match_from_lan {
                                if let Some(l) = lan.as_mut() {
                                    l.net.leave_match(m);
                                }
                                screen = Screen::LanLounge;
                            } else {
                                online.leave_match(m);
                                screen = Screen::Online;
                            }
                            current = None;
                            hud = Hud::new(config.show_help);
                            rig.reset();
                        }
                        HudAction::Settings => settings_open = true,
                        HudAction::ResetCamera => rig.reset(),
                        HudAction::Zoom(z) => rig.set_zoom_level(z),
                        HudAction::None => {
                            if m.is_online() {
                                let net = if match_from_lan {
                                    lan.as_mut().map(|l| &mut l.net)
                                } else {
                                    Some(&mut online)
                                };
                                if let Some(net) = net
                                    && net.pending_invites() > 0
                                {
                                    net.draw_invites(&mut ui, true);
                                }
                            }
                        }
                    }
                }
                None => {
                    let mut next: Option<Screen> = None;
                    match &mut screen {
                        Screen::Menu => match menu.draw(&mut ui, &online) {
                            MenuAction::Online => {
                                online.connect();
                                next = Some(Screen::Online);
                            }
                            MenuAction::VsComputer => {
                                next = Some(Screen::Setup(Setup::new(SetupKind::VsComputer)))
                            }
                            MenuAction::TwoPlayers => {
                                next = Some(Screen::Setup(Setup::new(SetupKind::TwoPlayers)))
                            }
                            MenuAction::Lan => {
                                lan_screen.get_or_insert_with(LanScreen::new);
                                next = Some(if lan.is_some() {
                                    Screen::LanLounge
                                } else {
                                    Screen::Lan
                                });
                            }
                            MenuAction::Ranking => {
                                ranking.open(&mut online);
                                back_to_lounge = false;
                                next = Some(Screen::Ranking);
                            }
                            MenuAction::Profile => {
                                back_to_lounge = false;
                                match online.me.clone() {
                                    Some(me) => {
                                        profile.open(&me.username, &mut online);
                                        next = Some(Screen::Profile);
                                    }
                                    None => {
                                        online.connect();
                                        next = Some(Screen::Online);
                                    }
                                }
                            }
                            MenuAction::Shop => {
                                shop.open(&mut online);
                                back_to_lounge = false;
                                next = Some(Screen::Shop);
                            }
                            MenuAction::Settings => settings_open = true,
                            MenuAction::Quit => break,
                            MenuAction::None => {}
                        },
                        Screen::Setup(setup) => match setup.draw(&mut ui, &mut config) {
                            SetupAction::Back => next = Some(Screen::Menu),
                            SetupAction::Start(settings) => {
                                config.save();
                                current = Some(Match::new(&settings, game::clock_seed()));
                                hud = Hud::new(config.show_help);
                                rig.reset();
                                next = Some(Screen::Menu);
                            }
                            SetupAction::None => {}
                        },
                        Screen::Online => match online.draw(&mut ui, &mut config) {
                            OnlineAction::Leave => next = Some(Screen::Menu),
                            OnlineAction::Profile(name) => {
                                profile.open(&name, &mut online);
                                back_to_lounge = true;
                                next = Some(Screen::Profile);
                            }
                            OnlineAction::Ranking => {
                                ranking.open(&mut online);
                                back_to_lounge = true;
                                next = Some(Screen::Ranking);
                            }
                            OnlineAction::Shop => {
                                shop.open(&mut online);
                                back_to_lounge = true;
                                next = Some(Screen::Shop);
                            }
                            OnlineAction::None => {}
                        },
                        Screen::Lan => {
                            let ls = lan_screen.get_or_insert_with(LanScreen::new);
                            let hosting =
                                lan.as_ref().and_then(|l| l.host.as_ref()).map(|h| h.port);
                            match ls.draw(&mut ui, &mut config, hosting) {
                                LanAction::Back => next = Some(Screen::Menu),
                                LanAction::Host(name) => {
                                    if let Some((host, url)) = ls.host(&name) {
                                        lan = Some(LanSession {
                                            host: Some(host),
                                            net: Online::lan(&url, &name),
                                        });
                                        config.save();
                                        next = Some(Screen::LanLounge);
                                    }
                                }
                                LanAction::Join(url, name) => {
                                    lan = Some(LanSession {
                                        host: None,
                                        net: Online::lan(&url, &name),
                                    });
                                    config.save();
                                    next = Some(Screen::LanLounge);
                                }
                                LanAction::None => {}
                            }
                        }
                        Screen::LanLounge => match lan.as_mut() {
                            Some(l) => match l.net.draw(&mut ui, &mut config) {
                                OnlineAction::Leave => {
                                    // Leaving a LAN game closes it (and stops hosting).
                                    lan = None;
                                    next = Some(Screen::Menu);
                                }
                                OnlineAction::Profile(name) => {
                                    l.net.request_profile(&name);
                                }
                                _ => {}
                            },
                            None => next = Some(Screen::Lan),
                        },
                        Screen::Ranking => match ranking.draw(&mut ui, &mut online) {
                            RankingAction::Back => {
                                next =
                                    Some(if back_to_lounge { Screen::Online } else { Screen::Menu })
                            }
                            RankingAction::Profile(name) => {
                                profile.open(&name, &mut online);
                                next = Some(Screen::Profile);
                            }
                            RankingAction::None => {}
                        },
                        Screen::Shop if shop_preview.is_some() => {
                            let (code, _) = shop_preview.expect("checked");
                            match shop.preview_bar(&mut ui, &mut online, code) {
                                shop::PreviewAction::None => {}
                                shop::PreviewAction::Close => {
                                    shop.show_item(code);
                                    shop_preview = None;
                                }
                                shop::PreviewAction::Buy => {
                                    shop.show_item(code);
                                    shop.ask_to_buy(code);
                                    shop_preview = None;
                                }
                            }
                        }
                        Screen::Shop => match shop.draw(&mut ui, &mut online) {
                            shop::ShopAction::Back => {
                                next = Some(if back_to_lounge && online.signed_in() {
                                    Screen::Online
                                } else {
                                    Screen::Menu
                                })
                            }
                            shop::ShopAction::Preview(code) => {
                                shop_preview = Some((code, get_time() as f32));
                            }
                            shop::ShopAction::None => {}
                        },
                        Screen::Profile => match profile.draw(&mut ui, &mut online) {
                            ProfileAction::Back => {
                                next = Some(if back_to_lounge && online.signed_in() {
                                    Screen::Online
                                } else {
                                    Screen::Menu
                                })
                            }
                            ProfileAction::None => {}
                        },
                    }
                    if let Some(n) = next {
                        screen = n;
                    }
                }
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
                        Scene::new(
                            Table::default(),
                            q,
                            config.focused_scene,
                            &scene::Looks::of(&config.cloth, &config.balls),
                        ),
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
            let r = Rect::new((ui::vw() - w) / 2.0, ui::vh() - 60.0, w, 34.0);
            ui::rrect(r, 17.0, Color::new(0.0, 0.0, 0.0, 0.7 * alpha));
            ui.text_centered(
                text,
                ui::vw() / 2.0,
                r.y + 9.0,
                15.0,
                Color::new(1.0, 0.95, 0.85, alpha),
                false,
            );
            if *t <= 0.0 {
                toast = None;
            }
        }
        // The round trip to the server, discreet, during online matches.
        if config.show_ping && current.as_ref().is_some_and(|m| m.is_online()) && !settings_open {
            let rtt =
                if match_from_lan { lan.as_ref().and_then(|l| l.net.rtt) } else { online.rtt };
            online::draw_ping(&ui, rtt, ui::vw() - 12.0, 64.0);
        }
        // Achievements, friend requests, declined invitations…
        let toast_bottom = if current.is_some() { ui::vh() - 92.0 } else { ui::vh() - 20.0 };
        if current.is_none() && !settings_open && online.draw_email_offer(&mut ui, &mut config) {
            config.save();
        }
        online.draw_toasts(&ui, toast_bottom);
        if let Some(l) = &lan {
            l.net.draw_toasts(&ui, toast_bottom);
        }

        // Hover hints go on top of everything (panels, chat, toasts).
        ui.draw_overlays();

        let in_match = current.is_some();
        if in_match != was_in_match {
            was_in_match = in_match;
            fade = 0.85;
        }
        if fade > 0.0 {
            draw_rectangle(0.0, 0.0, ui::vw(), ui::vh(), Color::new(0.0, 0.0, 0.0, fade));
            fade -= dt * 2.2;
        }

        if config.show_fps || options.show_fps {
            let text = format!("{} FPS", get_fps());
            let w = ui.text_size(&text, 13.0, true).x;
            ui.text(&text, ui::vw() - w - 10.0, ui::vh() - 20.0, 13.0, ui::MUTED, true);
        }
        frame += 1;
        if options.show_fps && frame.is_multiple_of(120) {
            println!("fps {}", get_fps());
        }
        // Development aid: SNOOK_SHOTS="300,900" also saves `<name>-300.png`… on the way.
        if let (Some(path), Some(list)) = (&options.screenshot, crate::dev::var("SNOOK_SHOTS"))
            && list.split(',').filter_map(|n| n.trim().parse::<u32>().ok()).any(|n| n == frame)
        {
            let mut image = get_screen_data();
            for px in image.bytes.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
            image.export_png(&path.replace(".png", &format!("-{frame}.png")));
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
        if profiling {
            let ms = frame_start.elapsed().as_secs_f64() * 1000.0;
            cpu = (cpu.0 + ms, cpu.1.max(ms), cpu.2 + 1);
            if cpu.2 == 300 {
                eprintln!("cpu per frame: avg {:.2} ms, worst {:.2} ms", cpu.0 / 300.0, cpu.1);
                cpu = (0.0, 0.0, 0);
            }
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

/// Draws the in-game HUD, fading it away while the player aims.
fn hud_draw(
    hud: &mut Hud,
    ui: &mut Ui,
    m: &mut Match,
    aiming: bool,
    opacity: &mut f32,
    dt: f32,
) -> HudAction {
    let target = if aiming { 0.0 } else { 1.0 };
    let speed = if target < *opacity { 7.0 } else { 5.0 };
    *opacity += (target - *opacity) * (1.0 - (-dt * speed).exp());
    paint::set_opacity(*opacity);
    let action = if *opacity < 0.6 {
        let saved = ui.suspend_input();
        let a = hud.draw(ui, m);
        ui.resume_input(saved);
        a
    } else {
        hud.draw(ui, m)
    };
    paint::set_opacity(1.0);
    action
}
