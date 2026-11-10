//! A small immediate-mode UI toolkit drawn with macroquad shapes and a TTF font.

use macroquad::prelude::*;

pub const GOLD: Color = Color::new(0.91, 0.75, 0.38, 1.0);
pub const CREAM: Color = Color::new(0.94, 0.89, 0.80, 1.0);
pub const MUTED: Color = Color::new(0.70, 0.64, 0.56, 1.0);
pub const PANEL: Color = Color::new(0.09, 0.055, 0.035, 0.88);
pub const BORDER: Color = Color::new(0.45, 0.31, 0.16, 1.0);
pub const ALERT: Color = Color::new(1.0, 0.56, 0.44, 1.0);
pub const GREEN: Color = Color::new(0.12, 0.45, 0.25, 1.0);

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
        }
    }

    /// Call once at the start of each frame.
    pub fn begin(&mut self) {
        self.mouse = mouse_position().into();
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
        let mut s = size;
        while s > 10.0 && self.text_size(text, s, true).x > width {
            s -= 1.0;
        }
        s
    }

    pub fn text_size(&self, text: &str, size: f32, bold: bool) -> Vec2 {
        let font = if bold { &self.bold } else { &self.regular };
        let d = measure_text(text, Some(font), size as u16, 1.0);
        vec2(d.width, size)
    }

    /// Draws text with its top-left corner at (`x`, `y`).
    pub fn text(&self, text: &str, x: f32, y: f32, size: f32, color: Color, bold: bool) {
        let font = if bold { &self.bold } else { &self.regular };
        draw_text_ex(
            text,
            x.round(),
            (y + size * 0.8).round(),
            TextParams { font: Some(font), font_size: size as u16, color, ..Default::default() },
        );
    }

    pub fn text_centered(&self, text: &str, cx: f32, y: f32, size: f32, color: Color, bold: bool) {
        let w = self.text_size(text, size, bold).x;
        self.text(text, cx - w / 2.0, y, size, color, bold);
    }

    pub fn panel(&mut self, r: Rect) {
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
        let size = self.fit((r.h * 0.45).clamp(13.0, 22.0), label, r.w - 12.0);
        let color = if enabled { CREAM } else { MUTED };
        self.text_centered(label, r.x + r.w / 2.0, r.y + (r.h - size) / 2.0, size, color, true);
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
        let size = self.fit((r.h * 0.45).clamp(13.0, 24.0), label, r.w - 12.0);
        self.text_centered(label, r.x + r.w / 2.0, r.y + (r.h - size) / 2.0, size, WHITE, true);
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

    /// True while the left button is held on this button (for auto-repeat).
    pub fn hold_button(&mut self, id: u64, r: Rect, label: &str) -> bool {
        self.block(r);
        let hover = r.contains(self.mouse);
        if self.pressed && hover {
            self.active = Some(id);
        }
        let held = self.active == Some(id);
        let bg = if held {
            Color::new(0.50, 0.33, 0.15, 1.0)
        } else if hover {
            Color::new(0.38, 0.25, 0.12, 1.0)
        } else {
            Color::new(0.25, 0.16, 0.09, 1.0)
        };
        rrect(r, 6.0, bg);
        rrect_lines(r, 6.0, 1.2, if hover || held { GOLD } else { BORDER });
        let size = self.fit((r.h * 0.5).clamp(12.0, 20.0), label, r.w - 8.0);
        self.text_centered(label, r.x + r.w / 2.0, r.y + (r.h - size) / 2.0, size, CREAM, true);
        held
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
        if self.focus == Some(id) {
            let ctrl = is_key_down(KeyCode::LeftControl)
                || is_key_down(KeyCode::RightControl)
                || is_key_down(KeyCode::LeftSuper)
                || is_key_down(KeyCode::RightSuper);
            while let Some(c) = get_char_pressed() {
                if !c.is_control() && !ctrl && value.chars().count() < max_len {
                    value.push(c);
                }
            }
            if ctrl
                && is_key_pressed(KeyCode::V)
                && let Some(text) = macroquad::miniquad::window::clipboard_get()
            {
                for c in text.chars().filter(|c| !c.is_control()) {
                    if value.chars().count() >= max_len {
                        break;
                    }
                    value.push(c);
                }
            }
            if is_key_pressed(KeyCode::Backspace) {
                if ctrl {
                    value.clear();
                } else {
                    value.pop();
                }
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                submitted = true;
            }
            if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Tab) {
                self.focus = None;
            }
        }
        // Focus changes after typing is handled, so nothing typed is lost.
        if self.pressed {
            if r.contains(self.mouse) {
                if self.focus != Some(id) {
                    while get_char_pressed().is_some() {}
                }
                self.focus = Some(id);
            } else if self.focus == Some(id) {
                self.focus = None;
            }
        }
        let focused = self.focus == Some(id);
        rrect(r, 5.0, Color::new(0.04, 0.03, 0.02, 1.0));
        rrect_lines(r, 5.0, if focused { 1.6 } else { 1.2 }, if focused { GOLD } else { BORDER });
        let size = (r.h * 0.5).clamp(12.0, 18.0);
        let shown: String =
            if masked { "•".repeat(value.chars().count()) } else { value.clone() };
        // Keep the end of long text visible.
        let room = r.w - 20.0;
        let mut visible = shown.as_str();
        while !visible.is_empty() && self.text_size(visible, size, false).x > room {
            let mut it = visible.chars();
            it.next();
            visible = it.as_str();
        }
        self.text(visible, r.x + 8.0, r.y + (r.h - size) / 2.0, size, CREAM, false);
        if focused && (get_time() * 2.0) as i64 % 2 == 0 {
            let w = self.text_size(visible, size, false).x;
            draw_line(r.x + 10.0 + w, r.y + 6.0, r.x + 10.0 + w, r.y + r.h - 6.0, 1.5, CREAM);
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
        self.text_centered(
            label,
            r.x + r.w / 2.0,
            r.y + (r.h - 15.0) / 2.0,
            15.0,
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

    /// A scrollable chat log (newest at the bottom). `scroll` counts lines up
    /// from the bottom; the new value is returned.
    pub fn chat_log(
        &mut self,
        r: Rect,
        lines: &[snook_proto::ChatLine],
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
        let mut rows: Vec<(Option<(String, bool)>, String)> = Vec::new();
        for l in lines.iter().rev() {
            let mine = me.is_some_and(|m| m == l.from);
            let name = format!("{}: ", l.from);
            let name_w = self.text_size(&name, size, true).x;
            let wrapped = self.wrap(&l.text, (r.w - 16.0 - name_w).max(60.0), size);
            let mut block: Vec<(Option<(String, bool)>, String)> = wrapped
                .into_iter()
                .enumerate()
                .map(|(k, w)| (if k == 0 { Some((name.clone(), mine)) } else { None }, w))
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
        for (k, (name, text)) in rows[start..end].iter().enumerate() {
            let y = r.y + 4.0 + k as f32 * line_h;
            if let Some((name, mine)) = name {
                self.text(
                    name,
                    r.x + 8.0,
                    y,
                    size,
                    if *mine { GOLD } else { Color::new(0.55, 0.78, 0.95, 1.0) },
                    true,
                );
                indent = self.text_size(name, size, true).x;
            }
            self.text(text, r.x + 8.0 + indent, y, size, CREAM, false);
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

/// Outline points of a rounded rectangle, clockwise from the top-left corner.
fn rounded_points(r: Rect, radius: f32) -> Vec<Vec2> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    const STEPS: usize = 6;
    let corners = [
        (vec2(r.x + rad, r.y + rad), std::f32::consts::PI),
        (vec2(r.x + r.w - rad, r.y + rad), 1.5 * std::f32::consts::PI),
        (vec2(r.x + r.w - rad, r.y + r.h - rad), 0.0),
        (vec2(r.x + rad, r.y + r.h - rad), 0.5 * std::f32::consts::PI),
    ];
    let mut pts = Vec::with_capacity(4 * (STEPS + 1));
    for (c, start) in corners {
        for k in 0..=STEPS {
            let a = start + k as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
            pts.push(c + vec2(a.cos(), a.sin()) * rad);
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
