//! A small immediate-mode UI toolkit drawn with macroquad shapes and a TTF font.

use macroquad::prelude::*;

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_circle_lines, draw_line, draw_rectangle, draw_triangle};

pub const GOLD: Color = Color::new(0.91, 0.75, 0.38, 1.0);
pub const CREAM: Color = Color::new(0.94, 0.89, 0.80, 1.0);
pub const MUTED: Color = Color::new(0.70, 0.64, 0.56, 1.0);
pub const PANEL: Color = Color::new(0.09, 0.055, 0.035, 0.96);
pub const BORDER: Color = Color::new(0.45, 0.31, 0.16, 1.0);
pub const ALERT: Color = Color::new(1.0, 0.56, 0.44, 1.0);
pub const GREEN: Color = Color::new(0.12, 0.45, 0.25, 1.0);

/// A wrapped chat row: the coloured prefix (on its first row), the text, the kind.
type Row = (Option<(String, Color)>, String, FeedKind);

/// One line of a chat panel.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedLine {
    /// Who wrote it, with their rating (0: unknown) for the name's colour.
    pub from: Option<(String, i32)>,
    pub text: String,
    pub kind: FeedKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    /// A message from a player.
    Player,
    /// From the staff.
    Announcement,
    /// Something that happened in the game.
    Event,
    /// Something important that happened in the game (a foul…).
    Alert,
}

/// The colour of a rating tier (used for player names everywhere).
pub fn tier_color(tier: snookered_proto::Tier) -> Color {
    use snookered_proto::Tier;
    match tier {
        Tier::Rookie => Color::new(0.72, 0.72, 0.74, 1.0),
        Tier::Amateur => Color::new(0.93, 0.89, 0.80, 1.0),
        Tier::Skilled => Color::new(0.45, 0.86, 0.52, 1.0),
        Tier::Expert => Color::new(0.42, 0.72, 1.0, 1.0),
        Tier::Master => Color::new(0.78, 0.56, 1.0, 1.0),
        Tier::Grandmaster => Color::new(1.0, 0.55, 0.28, 1.0),
        Tier::Legend => Color::new(1.0, 0.83, 0.30, 1.0),
    }
}

/// The colour of a player's name.
pub fn name_color(p: &snookered_proto::PlayerInfo) -> Color {
    tier_color(p.tier())
}

/// A VIP name style (two colours), if the player has one.
pub fn name_style(p: &snookered_proto::PlayerInfo) -> Option<(Color, Color)> {
    if !p.vip {
        return None;
    }
    match snookered_proto::shop::item(&p.name_style)?.look {
        snookered_proto::shop::Look::Name(Some((a, b))) => {
            Some((Color::from_rgba(a[0], a[1], a[2], 255), Color::from_rgba(b[0], b[1], b[2], 255)))
        }
        _ => None,
    }
}

/// A player's name in their colour (their VIP style, else their tier's).
pub fn draw_name(ui: &Ui, p: &snookered_proto::PlayerInfo, x: f32, y: f32, size: f32) {
    match name_style(p) {
        Some((a, b)) => ui.styled_text(&p.username, x, y, size, a, b, true),
        None => ui.text(&p.username, x, y, size, name_color(p), true),
    }
}

/// State of a [`Ui::drag_area`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    Idle,
    Hover,
    /// Being dragged; the pointer position.
    Held(Vec2),
    /// The drag ended this frame, here.
    Released(Vec2),
}

static UI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// Size factor of the interface: 1.0 at 1280×760, larger on big screens,
/// smaller on small windows (times the player's own preference).
pub fn ui_scale() -> f32 {
    f32::from_bits(UI_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

/// Width of the screen in interface units.
pub fn vw() -> f32 {
    screen_width() / ui_scale()
}

/// Height of the screen in interface units.
pub fn vh() -> f32 {
    screen_height() / ui_scale()
}

/// Works out the interface scale for this frame. `preference` is the
/// player's setting (1.0 = automatic size).
pub fn update_scale(preference: f32) {
    let (sw, sh) = (screen_width().max(1.0), screen_height().max(1.0));
    // The layout is designed for about 1180×720 interface units.
    // Small windows scale everything down rather than let it overflow.
    let auto = (sw / 1180.0).min(sh / 720.0).clamp(0.45, 2.6);
    let scale = (auto * preference.clamp(0.7, 1.5)).max(0.42);
    UI_SCALE.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Draws the interface from here on in interface units (see [`vw`]).
pub fn begin_ui_camera() {
    let (w, h) = (vw(), vh());
    let cam = Camera2D {
        target: vec2(w / 2.0, h / 2.0),
        zoom: vec2(2.0 / w, 2.0 / h),
        ..Default::default()
    };
    set_camera(&cam);
}

/// Something drawn on top of the interface at the end of the frame.
struct Overlay {
    tooltip: bool,
    draw: Box<dyn FnOnce(&Ui)>,
}

pub struct Ui {
    regular: Font,
    bold: Font,
    mouse: Vec2,
    pressed: bool,
    released: bool,
    down: bool,
    /// The widget being dragged (sliders, the spin ball).
    active: Option<u64>,
    /// The widget whose drag ended this frame.
    released_active: Option<u64>,
    /// The text field with keyboard focus.
    pub focus: Option<u64>,
    /// True if the pointer is over a UI element this frame (blocks game input).
    pub hovered: bool,
    hovered_next: bool,
    /// UI rectangles drawn last frame (so a click is recognised as UI even
    /// when the pointer just moved there).
    blocks: Vec<Rect>,
    prev_blocks: Vec<Rect>,
    pub wants_keyboard: bool,
    /// Caret position in the focused field, in characters.
    cursor: usize,
    /// Ctrl+A in the focused field: the next key replaces everything.
    all_selected: bool,
    /// The editing key being held and when it repeats next.
    repeat: Option<(KeyCode, f64)>,
    /// The field `cursor` belongs to (focus can also be set from outside).
    cursor_owner: Option<u64>,
    /// Hover hints asked for this frame, with the opacity at the time. They
    /// are drawn by [`Ui::draw_overlays`] after everything else, so panels,
    /// chat and toasts drawn later in the frame never cover them.
    overlays: std::cell::RefCell<Vec<(Overlay, f32)>>,
    /// The panels drawn so far this frame: text that starts inside one never
    /// runs past its edge (see [`Ui::text`]).
    panels: std::cell::RefCell<Vec<Rect>>,
}

impl Ui {
    pub fn new() -> Self {
        let regular =
            load_ttf_font_from_bytes(include_bytes!("../assets/DejaVuSans.ttf")).expect("font");
        let bold = load_ttf_font_from_bytes(include_bytes!("../assets/DejaVuSans-Bold.ttf"))
            .expect("font");
        Ui {
            regular,
            bold,
            mouse: Vec2::ZERO,
            pressed: false,
            released: false,
            down: false,
            active: None,
            released_active: None,
            focus: None,
            hovered: false,
            hovered_next: false,
            blocks: Vec::new(),
            prev_blocks: Vec::new(),
            wants_keyboard: false,
            cursor: 0,
            all_selected: false,
            repeat: None,
            cursor_owner: None,
            overlays: std::cell::RefCell::new(Vec::new()),
            panels: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Call once at the start of each frame.
    pub fn begin(&mut self) {
        self.overlays.get_mut().clear();
        self.panels.get_mut().clear();
        self.mouse = Vec2::from(mouse_position()) / ui_scale();
        self.pressed = is_mouse_button_pressed(MouseButton::Left);
        self.released = is_mouse_button_released(MouseButton::Left);
        self.down = is_mouse_button_down(MouseButton::Left);
        self.prev_blocks = std::mem::take(&mut self.blocks);
        let over_ui = self.prev_blocks.iter().any(|r| r.contains(self.mouse));
        self.hovered = over_ui || self.hovered_next || self.active.is_some();
        self.hovered_next = false;
        self.released_active = None;
        if !self.down {
            self.released_active = self.active.take();
        }
        self.wants_keyboard = self.focus.is_some();
        if self.focus.is_none() {
            self.cursor_owner = None;
        }
    }

    /// Marks a rectangle as UI so clicks there don't reach the game.
    pub fn block(&mut self, r: Rect) {
        self.blocks.push(r);
        if r.contains(self.mouse) {
            self.hovered_next = true;
        }
    }

    /// The largest size up to `size` at which bold `text` fits in `width`.
    pub fn fit(&self, size: f32, text: &str, width: f32) -> f32 {
        self.fit_with(size, text, width, true, 10.0)
    }

    fn fit_with(&self, size: f32, text: &str, width: f32, bold: bool, min: f32) -> f32 {
        let mut s = size;
        while s > min && self.text_size(text, s, bold).x > width {
            s = (s - 1.0).max(min);
        }
        s
    }

    /// A label that fits `width`: smaller type first, then cut with "…".
    pub fn fitted(&self, text: &str, size: f32, width: f32, bold: bool) -> (String, f32) {
        self.fitted_with(text, size, width, bold, 10.0)
    }

    fn fitted_with(
        &self,
        text: &str,
        size: f32,
        width: f32,
        bold: bool,
        min: f32,
    ) -> (String, f32) {
        let size = self.fit_with(size, text, width, bold, min.min(size));
        if self.text_size(text, size, bold).x <= width {
            return (text.to_string(), size);
        }
        let mut s: String = text.to_string();
        while !s.is_empty() && self.text_size(&format!("{s}…"), size, bold).x > width {
            s.pop();
        }
        (format!("{}…", s.trim_end()), size)
    }

    pub fn text_size(&self, text: &str, size: f32, bold: bool) -> Vec2 {
        let font = if bold { &self.bold } else { &self.regular };
        // Fonts are rasterised at the real pixel size, so text stays sharp at
        // any interface scale.
        let k = ui_scale();
        let d = measure_text(text, Some(font), (size * k).round().max(1.0) as u16, 1.0 / k);
        vec2(d.width, size)
    }

    /// The smallest panel drawn this frame that contains `p`.
    fn panel_at(&self, p: Vec2) -> Option<Rect> {
        self.panels
            .borrow()
            .iter()
            .filter(|r| r.contains(p))
            .min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))
            .copied()
    }

    /// Text that would run `width` past the room it has is made smaller
    /// (down to 70%), then cut with "…". Returns what to draw, its size and
    /// how far down to move it so it stays centred on the same line.
    fn shrink_to(&self, text: &str, size: f32, room: f32, bold: bool) -> Option<(String, f32)> {
        if room <= 0.0 || self.text_size(text, size, bold).x <= room {
            return None;
        }
        Some(self.fitted_with(text, size, room, bold, (size * 0.7).max(9.0)))
    }

    /// Draws text with its top-left corner at (`x`, `y`). Text that starts
    /// inside a panel never runs past the panel's right edge.
    pub fn text(&self, text: &str, x: f32, y: f32, size: f32, color: Color, bold: bool) {
        if let Some(r) = self.panel_at(vec2(x + 0.5, y + size * 0.5)) {
            let room = r.x + r.w - 6.0 - x;
            if let Some((t, s)) = self.shrink_to(text, size, room, bold) {
                self.draw_text_raw(&t, x, y + (size - s) * 0.5, s, color, bold);
                return;
            }
        }
        self.draw_text_raw(text, x, y, size, color, bold);
    }

    fn draw_text_raw(&self, text: &str, x: f32, y: f32, size: f32, color: Color, bold: bool) {
        let font = if bold { &self.bold } else { &self.regular };
        draw_text_ex(
            text,
            (x * ui_scale()).round() / ui_scale(),
            ((y + size * 0.8) * ui_scale()).round() / ui_scale(),
            TextParams {
                font: Some(font),
                font_size: (size * ui_scale()).round().max(1.0) as u16,
                font_scale: 1.0 / ui_scale(),
                color: crate::paint::fade(color),
                ..Default::default()
            },
        );
    }

    /// Text in two colours (a VIP name style): the second one as a soft
    /// glow around the first.
    #[allow(clippy::too_many_arguments)]
    pub fn styled_text(
        &self,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        a: Color,
        b: Color,
        bold: bool,
    ) {
        let glow = Color::new(b.r, b.g, b.b, 0.5);
        for (dx, dy) in [(-0.8, 0.0), (0.8, 0.0), (0.0, 0.8), (0.0, -0.8)] {
            self.text(text, x + dx, y + dy, size, glow, bold);
        }
        self.text(text, x, y, size, a, bold);
    }

    /// Text centred on `cx`; inside a panel it stays within the panel.
    pub fn text_centered(&self, text: &str, cx: f32, y: f32, size: f32, color: Color, bold: bool) {
        if let Some(r) = self.panel_at(vec2(cx, y + size * 0.5)) {
            let half = (cx - r.x).min(r.x + r.w - cx) - 8.0;
            if let Some((t, s)) = self.shrink_to(text, size, half * 2.0, bold) {
                let w = self.text_size(&t, s, bold).x;
                self.draw_text_raw(&t, cx - w / 2.0, y + (size - s) * 0.5, s, color, bold);
                return;
            }
        }
        let w = self.text_size(text, size, bold).x;
        self.draw_text_raw(text, cx - w / 2.0, y, size, color, bold);
    }

    pub fn panel(&mut self, r: Rect) {
        self.panels.get_mut().push(r);
        self.block(r);
        // Soft drop shadow, body, a faint top sheen and a thin gold-brown edge.
        rrect(
            Rect::new(r.x - 2.0, r.y + 3.0, r.w + 4.0, r.h + 5.0),
            10.0,
            Color::new(0.0, 0.0, 0.0, 0.28),
        );
        rrect(r, 8.0, PANEL);
        rrect(
            Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, (r.h * 0.5).min(40.0)),
            7.0,
            Color::new(1.0, 0.85, 0.6, 0.035),
        );
        rrect_lines(r, 8.0, 1.2, BORDER);
    }

    pub fn button(&mut self, r: Rect, label: &str, enabled: bool) -> bool {
        self.block(r);
        let hover = enabled && r.contains(self.mouse);
        let bg = if !enabled {
            Color::new(0.16, 0.12, 0.09, 0.9)
        } else if hover && self.down {
            Color::new(0.50, 0.33, 0.15, 1.0)
        } else if hover {
            Color::new(0.38, 0.25, 0.12, 1.0)
        } else {
            Color::new(0.25, 0.16, 0.09, 1.0)
        };
        rrect(r, 6.0, bg);
        if enabled {
            rrect(
                Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h * 0.45),
                5.0,
                Color::new(1.0, 0.9, 0.7, 0.06),
            );
        }
        rrect_lines(r, 6.0, 1.2, if hover { GOLD } else { BORDER });
        let (label, size) = self.fitted(label, (r.h * 0.45).clamp(13.0, 22.0), r.w - 12.0, true);
        let color = if enabled { CREAM } else { MUTED };
        self.text_centered(&label, r.x + r.w / 2.0, r.y + (r.h - size) / 2.0, size, color, true);
        enabled && hover && self.released
    }

    /// A button styled as the main call to action.
    pub fn primary_button(&mut self, r: Rect, label: &str) -> bool {
        self.block(r);
        let hover = r.contains(self.mouse);
        let bg = if hover && self.down {
            Color::new(0.10, 0.40, 0.22, 1.0)
        } else if hover {
            Color::new(0.18, 0.58, 0.33, 1.0)
        } else {
            GREEN
        };
        rrect(Rect::new(r.x, r.y + 2.0, r.w, r.h), 7.0, Color::new(0.0, 0.0, 0.0, 0.3));
        rrect(r, 7.0, bg);
        rrect(
            Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h * 0.45),
            6.0,
            Color::new(1.0, 1.0, 1.0, 0.08),
        );
        rrect_lines(r, 7.0, 1.2, if hover { GOLD } else { Color::new(0.3, 0.7, 0.45, 1.0) });
        let (label, size) = self.fitted(label, (r.h * 0.45).clamp(13.0, 24.0), r.w - 12.0, true);
        self.text_centered(&label, r.x + r.w / 2.0, r.y + (r.h - size) / 2.0, size, WHITE, true);
        hover && self.released
    }

    pub fn checkbox(&mut self, x: f32, y: f32, label: &str, value: &mut bool) -> bool {
        let r = Rect::new(x, y, 20.0 + self.text_size(label, 15.0, false).x + 10.0, 20.0);
        self.block(r);
        let hover = r.contains(self.mouse);
        let b = Rect::new(x, y, 18.0, 18.0);
        rrect(b, 4.0, Color::new(0.06, 0.04, 0.03, 1.0));
        rrect_lines(b, 4.0, 1.2, if hover { GOLD } else { BORDER });
        if *value {
            // A check mark.
            draw_line(x + 4.0, y + 9.5, x + 7.5, y + 13.0, 2.2, GOLD);
            draw_line(x + 7.5, y + 13.0, x + 14.0, y + 5.0, 2.2, GOLD);
        }
        self.text(label, x + 26.0, y + 1.0, 15.0, CREAM, false);
        if hover && self.released {
            *value = !*value;
            return true;
        }
        false
    }

    pub fn radio(&mut self, x: f32, y: f32, label: &str, selected: bool) -> bool {
        let r = Rect::new(x, y, 24.0 + self.text_size(label, 15.0, false).x, 20.0);
        self.block(r);
        let hover = r.contains(self.mouse);
        draw_circle(x + 9.0, y + 9.0, 8.0, Color::new(0.06, 0.04, 0.03, 1.0));
        draw_circle_lines(x + 9.0, y + 9.0, 8.0, 1.5, if hover { GOLD } else { BORDER });
        if selected {
            draw_circle(x + 9.0, y + 9.0, 4.5, GOLD);
        }
        self.text(label, x + 24.0, y + 1.0, 15.0, CREAM, false);
        hover && self.released
    }

    /// Horizontal slider for `value` in `[min, max]`.
    pub fn slider(&mut self, id: u64, r: Rect, value: &mut f32, min: f32, max: f32) -> bool {
        self.block(r);
        if r.contains(self.mouse) && self.pressed {
            self.active = Some(id);
        }
        let mut changed = false;
        if self.active == Some(id) {
            let t = ((self.mouse.x - r.x) / r.w).clamp(0.0, 1.0);
            let v = min + t * (max - min);
            changed = v != *value;
            *value = v;
        }
        let t = ((*value - min) / (max - min)).clamp(0.0, 1.0);
        let cy = r.y + r.h / 2.0;
        rrect(Rect::new(r.x, cy - 3.0, r.w, 6.0), 3.0, Color::new(0.06, 0.04, 0.03, 1.0));
        if t > 0.0 {
            rrect(Rect::new(r.x, cy - 3.0, (r.w * t).max(6.0), 6.0), 3.0, GOLD);
        }
        draw_circle(r.x + r.w * t, cy, 8.0, CREAM);
        changed
    }

    /// A press-and-drag area: returns what the pointer is doing with it.
    pub fn drag_area(&mut self, id: u64, r: Rect) -> Drag {
        self.block(r);
        if self.pressed && r.contains(self.mouse) {
            self.active = Some(id);
        }
        if self.active == Some(id) {
            Drag::Held(self.mouse)
        } else if self.released_active == Some(id) {
            Drag::Released(self.mouse)
        } else if r.contains(self.mouse) {
            Drag::Hover
        } else {
            Drag::Idle
        }
    }

    /// While the HUD is faded out it must not react to the pointer (a drag
    /// that ends over a hidden button is not a click). Returns what to give
    /// back to [`Ui::resume_input`].
    pub fn suspend_input(&mut self) -> (Vec2, bool, bool, bool) {
        let saved = (self.mouse, self.pressed, self.released, self.down);
        self.mouse = vec2(-1.0e6, -1.0e6);
        self.pressed = false;
        self.released = false;
        self.down = false;
        saved
    }

    pub fn resume_input(&mut self, saved: (Vec2, bool, bool, bool)) {
        (self.mouse, self.pressed, self.released, self.down) = saved;
    }

    pub fn mouse(&self) -> Vec2 {
        self.mouse
    }

    /// Single-line text input. Returns true when Enter is pressed.
    pub fn text_input(&mut self, id: u64, r: Rect, value: &mut String, max_len: usize) -> bool {
        self.input(id, r, value, max_len, false)
    }

    /// Like [`Ui::text_input`], but shows dots instead of the characters.
    pub fn password_input(&mut self, id: u64, r: Rect, value: &mut String, max_len: usize) -> bool {
        self.input(id, r, value, max_len, true)
    }

    /// True when `key` was pressed this frame or is held long enough to
    /// repeat (macroquad reports only the first press).
    fn key_fires(&mut self, key: KeyCode) -> bool {
        let now = get_time();
        if is_key_pressed(key) {
            self.repeat = Some((key, now + 0.45));
            return true;
        }
        match self.repeat {
            Some((k, next)) if k == key && is_key_down(key) && now >= next => {
                self.repeat = Some((key, now + 0.035));
                true
            }
            Some((k, _)) if k == key && !is_key_down(key) => {
                self.repeat = None;
                false
            }
            _ => false,
        }
    }

    fn input(
        &mut self,
        id: u64,
        r: Rect,
        value: &mut String,
        max_len: usize,
        masked: bool,
    ) -> bool {
        self.block(r);
        let mut submitted = false;
        let size = (r.h * 0.5).clamp(12.0, 18.0);
        if self.focus == Some(id) && self.cursor_owner != Some(id) {
            self.cursor_owner = Some(id);
            self.cursor = value.chars().count();
            self.all_selected = false;
        }
        if self.focus == Some(id) {
            let ctrl = is_key_down(KeyCode::LeftControl)
                || is_key_down(KeyCode::RightControl)
                || is_key_down(KeyCode::LeftSuper)
                || is_key_down(KeyCode::RightSuper);
            let mut chars: Vec<char> = value.chars().collect();
            let mut cursor = self.cursor.min(chars.len());
            let mut changed = false;
            let mut typed: Vec<char> = Vec::new();
            while let Some(c) = get_char_pressed() {
                if !c.is_control() && !ctrl {
                    typed.push(c);
                }
            }
            if ctrl
                && is_key_pressed(KeyCode::V)
                && let Some(text) = macroquad::miniquad::window::clipboard_get()
            {
                typed.extend(text.chars().filter(|c| !c.is_control()));
            }
            if ctrl && is_key_pressed(KeyCode::A) {
                self.all_selected = !chars.is_empty();
                cursor = chars.len();
            }
            if ctrl && is_key_pressed(KeyCode::C) && !masked {
                macroquad::miniquad::window::clipboard_set(value);
            }
            let erase_back = self.key_fires(KeyCode::Backspace);
            let erase_fwd = self.key_fires(KeyCode::Delete);
            if self.all_selected && (erase_back || erase_fwd || !typed.is_empty()) {
                chars.clear();
                cursor = 0;
                changed = true;
                self.all_selected = false;
            } else if erase_back && cursor > 0 {
                let from = if ctrl { word_start(&chars, cursor) } else { cursor - 1 };
                chars.drain(from..cursor);
                cursor = from;
                changed = true;
            } else if erase_fwd && cursor < chars.len() {
                let to = if ctrl { word_end(&chars, cursor) } else { cursor + 1 };
                chars.drain(cursor..to);
                changed = true;
            }
            for c in typed {
                if chars.len() >= max_len {
                    break;
                }
                chars.insert(cursor, c);
                cursor += 1;
                changed = true;
            }
            let mut moved = false;
            if self.key_fires(KeyCode::Left) {
                cursor = if ctrl { word_start(&chars, cursor) } else { cursor.saturating_sub(1) };
                moved = true;
            }
            if self.key_fires(KeyCode::Right) {
                cursor =
                    if ctrl { word_end(&chars, cursor) } else { (cursor + 1).min(chars.len()) };
                moved = true;
            }
            if is_key_pressed(KeyCode::Home) || is_key_pressed(KeyCode::Up) {
                cursor = 0;
                moved = true;
            }
            if is_key_pressed(KeyCode::End) || is_key_pressed(KeyCode::Down) {
                cursor = chars.len();
                moved = true;
            }
            if moved {
                self.all_selected = false;
            }
            if changed {
                *value = chars.into_iter().collect();
            }
            self.cursor = cursor;
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                submitted = true;
                self.all_selected = false;
            }
            if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Tab) {
                self.focus = None;
            }
        }
        let shown: Vec<char> =
            if masked { vec!['•'; value.chars().count()] } else { value.chars().collect() };
        // Scroll so the caret stays visible: as much text as fits, ending
        // at the caret when the caret would be off the right edge.
        let room = r.w - 20.0;
        let caret = if self.focus == Some(id) { self.cursor.min(shown.len()) } else { shown.len() };
        let width_of = |ui: &Ui, a: usize, b: usize| {
            ui.text_size(&shown[a..b].iter().collect::<String>(), size, false).x
        };
        let mut first = 0;
        while first < caret && width_of(self, first, caret) > room {
            first += 1;
        }
        let mut last = caret;
        while last < shown.len() && width_of(self, first, last + 1) <= room {
            last += 1;
        }
        // Focus changes after typing is handled, so nothing typed is lost.
        if self.pressed {
            if r.contains(self.mouse) {
                if self.focus != Some(id) {
                    while get_char_pressed().is_some() {}
                    self.focus = Some(id);
                    self.cursor_owner = Some(id);
                    self.cursor = shown.len();
                    self.all_selected = false;
                } else {
                    // Put the caret where the click was.
                    let mx = self.mouse.x - (r.x + 8.0);
                    let mut best = first;
                    for i in first..=last {
                        if width_of(self, first, i) <= mx + 3.0 {
                            best = i;
                        }
                    }
                    self.cursor = best;
                    self.all_selected = false;
                }
            } else if self.focus == Some(id) {
                self.focus = None;
            }
        }
        let focused = self.focus == Some(id);
        rrect(r, 5.0, Color::new(0.04, 0.03, 0.02, 1.0));
        rrect_lines(r, 5.0, if focused { 1.6 } else { 1.2 }, if focused { GOLD } else { BORDER });
        let visible: String = shown[first..last].iter().collect();
        if focused && self.all_selected && !visible.is_empty() {
            let w = self.text_size(&visible, size, false).x;
            draw_rectangle(
                r.x + 7.0,
                r.y + 5.0,
                w + 2.0,
                r.h - 10.0,
                Color::new(0.75, 0.55, 0.2, 0.45),
            );
        }
        self.text(&visible, r.x + 8.0, r.y + (r.h - size) / 2.0, size, CREAM, false);
        if focused && (get_time() * 2.0) as i64 % 2 == 0 {
            let caret = self.cursor.min(shown.len()).clamp(first, last);
            let w = width_of(self, first, caret);
            draw_line(r.x + 9.0 + w, r.y + 6.0, r.x + 9.0 + w, r.y + r.h - 6.0, 1.5, CREAM);
        }
        submitted
    }

    /// A square toggle button with no label (the caller draws its content);
    /// returns true when clicked.
    pub fn toggle(&mut self, r: Rect, on: bool) -> bool {
        self.block(r);
        let hover = r.contains(self.mouse);
        let bg = if on {
            Color::new(0.38, 0.25, 0.12, 1.0)
        } else if hover {
            Color::new(0.25, 0.16, 0.09, 1.0)
        } else {
            Color::new(0.12, 0.08, 0.05, 1.0)
        };
        rrect(r, 8.0, bg);
        rrect_lines(r, 8.0, if on { 1.8 } else { 1.2 }, if hover || on { GOLD } else { BORDER });
        hover && self.released
    }

    /// A tab header; returns true when clicked.
    pub fn tab(&mut self, r: Rect, label: &str, selected: bool) -> bool {
        self.tab_with_count(r, label, "", selected)
    }

    /// A tab with a small count at its right end (e.g. "3/14"); the label
    /// keeps clear of it, shrinking if it must.
    pub fn tab_with_count(&mut self, r: Rect, label: &str, count: &str, selected: bool) -> bool {
        self.block(r);
        let hover = r.contains(self.mouse);
        let bg = if selected {
            Color::new(0.38, 0.25, 0.12, 1.0)
        } else if hover {
            Color::new(0.25, 0.16, 0.09, 1.0)
        } else {
            Color::new(0.12, 0.08, 0.05, 1.0)
        };
        rrect(r, 6.0, bg);
        if selected {
            draw_rectangle(r.x + 8.0, r.y + r.h - 3.0, r.w - 16.0, 3.0, GOLD);
        }
        // Room for the count on the right; the label is centred when that
        // clears it, else moved left (and shrunk only if it must).
        let reserve = if count.is_empty() {
            0.0
        } else {
            let cw = self.text_size(count, 11.0, false).x;
            self.text(count, r.x + r.w - cw - 10.0, r.y + (r.h - 11.0) / 2.0, 11.0, MUTED, false);
            cw + 14.0
        };
        let (label, size) = self.fitted(label, 15.0, r.w - 16.0 - reserve, true);
        let width = self.text_size(&label, size, true).x;
        let centre = (r.x + r.w / 2.0).min(r.x + r.w - reserve - 6.0 - width / 2.0);
        self.text_centered(
            &label,
            centre,
            r.y + (r.h - size) / 2.0,
            size,
            if selected { GOLD } else { CREAM },
            true,
        );
        hover && self.released && !selected
    }

    /// Splits `text` into lines no wider than `width`.
    pub fn wrap(&self, text: &str, width: f32, size: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate =
                if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.text_size(&candidate, size, false).x <= width || line.is_empty() {
                line = candidate;
                // A single word longer than the line: cut it.
                while self.text_size(&line, size, false).x > width && line.chars().count() > 1 {
                    let mut head = String::new();
                    for c in line.chars() {
                        if self.text_size(&format!("{head}{c}"), size, false).x > width {
                            break;
                        }
                        head.push(c);
                    }
                    if head.is_empty() {
                        break;
                    }
                    let rest = line[head.len()..].to_string();
                    lines.push(head);
                    line = rest;
                }
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    /// Shortens `text` with an ellipsis to fit `width`.
    pub fn ellipsize(&self, text: &str, width: f32, size: f32) -> String {
        if self.text_size(text, size, false).x <= width {
            return text.to_string();
        }
        let mut s: String = text.to_string();
        while !s.is_empty() && self.text_size(&format!("{s}…"), size, false).x > width {
            s.pop();
        }
        format!("{s}…")
    }

    /// A scrollable chat log (newest at the bottom) mixing player messages
    /// and what happened in the game. `scroll` counts lines up from the
    /// bottom; the new value is returned.
    pub fn chat_log(
        &mut self,
        r: Rect,
        lines: &[FeedLine],
        me: Option<&str>,
        scroll: usize,
    ) -> usize {
        self.block(r);
        let size = 14.0;
        let line_h = 19.0;
        let visible = ((r.h - 8.0) / line_h).floor().max(1.0) as usize;
        // Wrap only the newest messages that can be shown (measuring text is
        // the expensive part with a long history).
        let needed = visible + scroll + 1;
        let mut rows: Vec<Row> = Vec::new();
        for l in lines.iter().rev() {
            let (prefix, color) = match (&l.from, l.kind) {
                (Some((name, rating)), FeedKind::Player) => {
                    let mine = me.is_some_and(|m| m.eq_ignore_ascii_case(name));
                    let c = if *rating > 0 {
                        tier_color(snookered_proto::Tier::of(*rating))
                    } else {
                        Color::new(0.55, 0.78, 0.95, 1.0)
                    };
                    (Some(format!("{name}: ")), if mine { GOLD } else { c })
                }
                (_, FeedKind::Announcement) => (Some("★ ".to_string()), GOLD),
                _ => (Some("• ".to_string()), MUTED),
            };
            let bold = l.kind == FeedKind::Player;
            let name_w = prefix.as_ref().map(|p| self.text_size(p, size, bold).x).unwrap_or(0.0);
            let wrapped = self.wrap(&l.text, (r.w - 16.0 - name_w).max(60.0), size);
            let mut block: Vec<Row> = wrapped
                .into_iter()
                .enumerate()
                .map(|(k, w)| {
                    (if k == 0 { prefix.clone().map(|p| (p, color)) } else { None }, w, l.kind)
                })
                .collect();
            block.append(&mut rows);
            rows = block;
            if rows.len() >= needed + 20 {
                break;
            }
        }
        let max_scroll = rows.len().saturating_sub(visible);
        let mut scroll = scroll.min(max_scroll);
        if r.contains(self.mouse) {
            let (_, wheel) = mouse_wheel();
            if wheel > 0.0 {
                scroll = (scroll + 3).min(max_scroll);
            } else if wheel < 0.0 {
                scroll = scroll.saturating_sub(3);
            }
        }
        let end = rows.len() - scroll;
        let start = end.saturating_sub(visible);
        let mut indent = 0.0;
        for (k, (name, text, kind)) in rows[start..end].iter().enumerate() {
            let y = r.y + 4.0 + k as f32 * line_h;
            let bold = *kind == FeedKind::Player;
            if let Some((name, color)) = name {
                self.text(name, r.x + 8.0, y, size, *color, bold);
                indent = self.text_size(name, size, bold).x;
            }
            // Game events are quieter than what people write.
            let color = match kind {
                FeedKind::Player => CREAM,
                FeedKind::Announcement => Color::new(1.0, 0.9, 0.65, 1.0),
                FeedKind::Alert => Color::new(0.88, 0.58, 0.48, 0.95),
                FeedKind::Event => Color::new(0.70, 0.64, 0.56, 0.85),
            };
            let text_size =
                if matches!(kind, FeedKind::Event | FeedKind::Alert) { 13.0 } else { size };
            self.text(
                text,
                r.x + 8.0 + indent,
                y + (size - text_size) * 0.5,
                text_size,
                color,
                false,
            );
        }
        if scroll > 0 {
            let label = crate::i18n::t("▼ newer messages");
            let w = self.text_size(label, 12.0, false).x + 12.0;
            draw_rectangle(
                r.x + r.w - w - 6.0,
                r.y + r.h - 22.0,
                w,
                18.0,
                Color::new(0.0, 0.0, 0.0, 0.7),
            );
            self.text(label, r.x + r.w - w, r.y + r.h - 20.0, 12.0, GOLD, false);
        }
        if rows.is_empty() {
            self.text(crate::i18n::t("No messages yet."), r.x + 8.0, r.y + 6.0, 14.0, MUTED, false);
        }
        scroll
    }

    /// A square button with an icon; `tip` is shown while hovering.
    pub fn icon_button(
        &mut self,
        r: Rect,
        icon: crate::icons::Icon,
        tip: &str,
        enabled: bool,
    ) -> bool {
        self.icon_button_ex(r, icon, tip, enabled, false)
    }

    /// Like [`Ui::icon_button`]; `subtle` draws it without a background until hovered.
    pub fn icon_button_ex(
        &mut self,
        r: Rect,
        icon: crate::icons::Icon,
        tip: &str,
        enabled: bool,
        subtle: bool,
    ) -> bool {
        self.block(r);
        let hover = enabled && r.contains(self.mouse);
        let bg = if hover && self.down {
            Color::new(0.50, 0.33, 0.15, 1.0)
        } else if hover {
            Color::new(0.38, 0.25, 0.12, 0.95)
        } else if subtle {
            Color::new(0.05, 0.035, 0.025, 0.45)
        } else {
            Color::new(0.20, 0.13, 0.08, 0.95)
        };
        rrect(r, (r.h * 0.22).min(8.0), bg);
        if !subtle || hover {
            rrect_lines(r, (r.h * 0.22).min(8.0), 1.2, if hover { GOLD } else { BORDER });
        }
        let color = if !enabled {
            Color::new(0.5, 0.45, 0.4, 0.6)
        } else if hover {
            GOLD
        } else {
            CREAM
        };
        crate::icons::draw(icon, vec2(r.x + r.w / 2.0, r.y + r.h / 2.0), r.h * 0.52, color);
        if hover && !tip.is_empty() {
            self.tooltip(tip, r);
        }
        enabled && hover && self.released
    }

    /// A small dark label under (or above, near the bottom) `anchor`. It is
    /// drawn at the end of the frame, on top of the whole interface; only the
    /// last one asked for in a frame is shown.
    pub fn tooltip(&self, text: &str, anchor: Rect) {
        let text = text.to_string();
        let mut list = self.overlays.borrow_mut();
        list.retain(|(o, _)| !o.tooltip);
        list.push((
            Overlay {
                tooltip: true,
                draw: Box::new(move |ui: &Ui| {
                    let w = ui.text_size(&text, 13.0, true).x + 18.0;
                    let below = anchor.y + anchor.h + 30.0 < vh();
                    let y =
                        if below { anchor.y + anchor.h + 6.0 } else { (anchor.y - 30.0).max(2.0) };
                    let x =
                        (anchor.x + anchor.w / 2.0 - w / 2.0).clamp(4.0, (vw() - w - 4.0).max(4.0));
                    let r = Rect::new(x, y, w, 24.0);
                    rrect(r, 12.0, Color::new(0.0, 0.0, 0.0, 0.9));
                    rrect_lines(r, 12.0, 1.0, Color::new(BORDER.r, BORDER.g, BORDER.b, 0.7));
                    ui.text(&text, r.x + 9.0, r.y + 5.0, 13.0, CREAM, true);
                }),
            },
            crate::paint::opacity(),
        ));
    }

    /// Draws `f` at the end of the frame, over the whole interface (hover
    /// cards and hints that must never end up under a panel).
    pub fn overlay(&self, f: impl FnOnce(&Ui) + 'static) {
        self.overlays
            .borrow_mut()
            .push((Overlay { tooltip: false, draw: Box::new(f) }, crate::paint::opacity()));
    }

    /// Draws this frame's hover hints over everything drawn so far. Call
    /// once, after the rest of the interface (in interface units).
    pub fn draw_overlays(&self) {
        // Hints float above everything: they are not held to any panel.
        self.panels.borrow_mut().clear();
        let list = std::mem::take(&mut *self.overlays.borrow_mut());
        let before = crate::paint::opacity();
        for (o, alpha) in list {
            crate::paint::set_opacity(alpha);
            (o.draw)(self);
        }
        crate::paint::set_opacity(before);
    }

    /// A flag (if any) and a player name in their tier colour. Returns the width used.
    pub fn player_name(
        &self,
        p: &snookered_proto::PlayerInfo,
        x: f32,
        y: f32,
        size: f32,
        max_w: f32,
    ) -> f32 {
        let crown = if p.vip { size + 4.0 } else { 0.0 };
        let w = match name_style(p) {
            Some((a, b)) => {
                let fh = (size * 0.78).round();
                let mut used = 0.0;
                if crate::flags::draw(&p.country, x, y + (size - fh) / 2.0 + 1.0, fh) {
                    used = crate::flags::width(fh) + 6.0;
                }
                let name = self.ellipsize(&p.username, max_w - crown - used, size);
                self.styled_text(&name, x + used, y, size, a, b, true);
                used + self.text_size(&name, size, true).x
            }
            None => self.name_with_flag(
                &p.username,
                &p.country,
                name_color(p),
                x,
                y,
                size,
                max_w - crown,
            ),
        };
        // The shop frame: a thin line of its colours under the name.
        if let Some(snookered_proto::shop::Look::Frame { a, b, .. }) =
            snookered_proto::shop::item(&p.frame).map(|i| i.look)
            && p.frame != "frame_none"
        {
            let ca = Color::from_rgba(a[0], a[1], a[2], 220);
            let cb = Color::from_rgba(b[0], b[1], b[2], 220);
            crate::paint::draw_rectangle(x, y + size + 1.0, w * 0.5, 2.0, ca);
            crate::paint::draw_rectangle(x + w * 0.5, y + size + 1.0, w * 0.5, 2.0, cb);
        }
        if p.vip {
            // VIP members: a small gold crown after the name.
            crate::icons::draw(
                crate::icons::Icon::Crown,
                vec2(x + w + 4.0 + size * 0.45, y + size * 0.55),
                size * 0.8,
                GOLD,
            );
        }
        w + crown
    }

    /// A flag and a name in `color`. Returns the width used.
    #[allow(clippy::too_many_arguments)]
    pub fn name_with_flag(
        &self,
        name: &str,
        country: &str,
        color: Color,
        x: f32,
        y: f32,
        size: f32,
        max_w: f32,
    ) -> f32 {
        let fh = (size * 0.78).round();
        let mut used = 0.0;
        if crate::flags::draw(country, x, y + (size - fh) / 2.0 + 1.0, fh) {
            used = crate::flags::width(fh) + 6.0;
        }
        let name = self.ellipsize(name, max_w - used, size);
        self.text(&name, x + used, y, size, color, true);
        used + self.text_size(&name, size, true).x
    }

    /// A rounded pill with short text (a tag such as "rated").
    pub fn pill(&self, text: &str, x: f32, y: f32, color: Color) -> f32 {
        let w = self.text_size(text, 11.0, true).x + 12.0;
        let r = Rect::new(x, y, w, 17.0);
        rrect(r, 8.5, Color::new(color.r, color.g, color.b, 0.16));
        rrect_lines(r, 8.5, 1.0, Color::new(color.r, color.g, color.b, 0.6));
        self.text(text, x + 6.0, y + 2.5, 11.0, color, true);
        w
    }

    /// Scrolls a list area with the mouse wheel; returns the new first row.
    pub fn scroll_list(&mut self, area: Rect, first: usize, rows: usize, visible: usize) -> usize {
        self.block(area);
        let max = rows.saturating_sub(visible);
        let mut first = first.min(max);
        if area.contains(self.mouse) {
            let (_, wheel) = mouse_wheel();
            if wheel > 0.0 {
                first = first.saturating_sub(3);
            } else if wheel < 0.0 {
                first = (first + 3).min(max);
            }
        }
        if rows > visible && visible > 0 {
            // A thin scroll indicator on the right.
            let track = Rect::new(area.x + area.w - 3.0, area.y, 3.0, area.h);
            let h = (track.h * visible as f32 / rows as f32).max(16.0);
            let y = track.y + (track.h - h) * first as f32 / max.max(1) as f32;
            rrect(Rect::new(track.x, y, 3.0, h), 1.5, Color::new(0.91, 0.75, 0.38, 0.45));
        }
        first
    }

    /// True if the left button was released over `r` this frame (a click).
    pub fn clicked(&self, r: Rect) -> bool {
        self.released && r.contains(self.mouse)
    }

    pub fn hovering(&self, r: Rect) -> bool {
        r.contains(self.mouse)
    }

    /// A row of joined toggle buttons; returns the index clicked.
    pub fn segmented(&mut self, r: Rect, options: &[&str], selected: usize) -> Option<usize> {
        self.block(r);
        let n = options.len().max(1) as f32;
        let w = r.w / n;
        rrect(r, 7.0, Color::new(0.06, 0.04, 0.03, 1.0));
        let mut clicked = None;
        for (i, label) in options.iter().enumerate() {
            let cell = Rect::new(r.x + i as f32 * w, r.y, w, r.h);
            let hover = cell.contains(self.mouse);
            if i == selected {
                rrect(
                    Rect::new(cell.x + 2.0, cell.y + 2.0, cell.w - 4.0, cell.h - 4.0),
                    6.0,
                    Color::new(0.38, 0.25, 0.12, 1.0),
                );
            } else if hover {
                rrect(
                    Rect::new(cell.x + 2.0, cell.y + 2.0, cell.w - 4.0, cell.h - 4.0),
                    6.0,
                    Color::new(0.22, 0.15, 0.09, 1.0),
                );
            }
            let (label, size) = self.fitted(label, (r.h * 0.42).clamp(12.0, 16.0), w - 10.0, true);
            let color = if i == selected { GOLD } else { CREAM };
            self.text_centered(
                &label,
                cell.x + w / 2.0,
                cell.y + (r.h - size) / 2.0,
                size,
                color,
                true,
            );
            if hover && self.released && i != selected {
                clicked = Some(i);
            }
        }
        rrect_lines(r, 7.0, 1.2, BORDER);
        clicked
    }

    /// The circular "where does the tip hit the cue ball" control.
    pub fn spin_ball(&mut self, id: u64, center: Vec2, radius: f32, tip: &mut (f32, f32)) {
        let r = Rect::new(center.x - radius, center.y - radius, radius * 2.0, radius * 2.0);
        self.block(r);
        if self.pressed && center.distance(self.mouse) <= radius {
            self.active = Some(id);
        }
        if self.active == Some(id) {
            let mut d = (self.mouse - center) / radius;
            if d.length() > 1.0 {
                d = d.normalize();
            }
            *tip = (d.x, -d.y);
        }
        draw_circle(center.x, center.y, radius, Color::new(0.96, 0.94, 0.88, 1.0));
        draw_circle_lines(center.x, center.y, radius, 2.0, Color::new(0.5, 0.42, 0.33, 1.0));
        draw_circle_lines(center.x, center.y, radius * 0.5, 1.0, Color::new(0.8, 0.76, 0.68, 1.0));
        let faint = Color::new(0.8, 0.76, 0.68, 1.0);
        draw_line(center.x - radius, center.y, center.x + radius, center.y, 1.0, faint);
        draw_line(center.x, center.y - radius, center.x, center.y + radius, 1.0, faint);
        let dot = center + vec2(tip.0, -tip.1) * radius;
        draw_circle(dot.x, dot.y, 7.0, Color::new(0.78, 0.16, 0.16, 1.0));
        draw_circle_lines(dot.x, dot.y, 7.0, 1.5, Color::new(0.35, 0.04, 0.04, 1.0));
    }
}

const CORNER_STEPS: usize = 6;
const OUTLINE: usize = 4 * (CORNER_STEPS + 1);

/// Outline points of a rounded rectangle, clockwise from the top-left corner
/// (on the stack: these are drawn hundreds of times a frame).
fn rounded_points(r: Rect, radius: f32) -> [Vec2; OUTLINE] {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let corners = [
        (vec2(r.x + rad, r.y + rad), std::f32::consts::PI),
        (vec2(r.x + r.w - rad, r.y + rad), 1.5 * std::f32::consts::PI),
        (vec2(r.x + r.w - rad, r.y + r.h - rad), 0.0),
        (vec2(r.x + rad, r.y + r.h - rad), 0.5 * std::f32::consts::PI),
    ];
    let mut pts = [Vec2::ZERO; OUTLINE];
    let mut i = 0;
    for (c, start) in corners {
        for k in 0..=CORNER_STEPS {
            let a = start + k as f32 / CORNER_STEPS as f32 * std::f32::consts::FRAC_PI_2;
            pts[i] = c + vec2(a.cos(), a.sin()) * rad;
            i += 1;
        }
    }
    pts
}

/// A filled rounded rectangle (one non-overlapping fan, so it blends correctly
/// when translucent).
pub fn rrect(r: Rect, radius: f32, color: Color) {
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let pts = rounded_points(r, radius);
    let c = vec2(r.x + r.w / 2.0, r.y + r.h / 2.0);
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        draw_triangle(c, a, b, color);
    }
}

/// The outline of a rounded rectangle.
pub fn rrect_lines(r: Rect, radius: f32, thickness: f32, color: Color) {
    let pts = rounded_points(r, radius);
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        draw_line(a.x, a.y, b.x, b.y, thickness, color);
    }
}

/// Where the word before `i` starts (Ctrl+Left, Ctrl+Backspace).
fn word_start(chars: &[char], mut i: usize) -> usize {
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Where the word after `i` ends (Ctrl+Right, Ctrl+Delete).
fn word_end(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    i
}
