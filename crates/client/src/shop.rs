//! The shop: cloth, cues, chalk, ball sets, frames, name styles and winning
//! effects (for coins, with achievements or for VIP members), coin packs and
//! the VIP subscription. Items can be tried on the 3D table before buying,
//! and given to friends.
//!
//! The server is the authority: this screen only asks it to buy, use or
//! open a payment page, and shows what it answers.

use macroquad::prelude::*;
use snookered_proto::ClientMsg;
use snookered_proto::shop::{Category, ITEMS, Item, Look, Price};

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_circle_lines, draw_line, draw_rectangle};

use crate::i18n::{t, tw};
use crate::icons::Icon;
use crate::online::Online;
use crate::ui::{ALERT, BORDER, CREAM, GOLD, MUTED, Ui, rrect, rrect_lines};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Items(Category),
    Coins,
    Vip,
}

/// What the 3D preview's bar asks for.
pub enum PreviewAction {
    None,
    Close,
    Buy,
}

pub enum ShopAction {
    None,
    Back,
    /// Try an item on the 3D table.
    Preview(&'static str),
}

/// A purchase waiting for the player to confirm it.
#[derive(Debug, Clone)]
struct Confirm {
    code: &'static str,
    /// Bought as a present for this friend.
    gift_to: Option<String>,
}

pub struct ShopScreen {
    tab: Tab,
    first: usize,
    asked: bool,
    confirm: Option<Confirm>,
    /// Choosing the friend who gets this item.
    gift_pick: Option<&'static str>,
    /// The purchase that just went through: (item, given to).
    done: Option<(&'static str, Option<String>)>,
}

impl Default for ShopScreen {
    fn default() -> Self {
        ShopScreen {
            tab: Tab::Items(Category::Cloth),
            first: 0,
            asked: false,
            confirm: None,
            gift_pick: None,
            done: None,
        }
    }
}

fn rgb(c: [u8; 3]) -> Color {
    Color::from_rgba(c[0], c[1], c[2], 255)
}

pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Cloth => t("Cloth"),
        Category::Cue => t("Cues"),
        Category::Chalk => t("Chalk"),
        Category::Balls => t("Balls"),
        Category::Frame => t("Frames"),
        Category::Name => t("Name colour"),
        Category::Effect => t("Winning effects"),
    }
}

/// Items that can be tried on the 3D table.
pub fn has_3d_preview(item: &Item) -> bool {
    matches!(item.category, Category::Cloth | Category::Balls | Category::Cue | Category::Chalk)
}

/// A frame of two colours (and a moving shine) around `r`.
pub fn frame(r: Rect, a: [u8; 3], b: [u8; 3], shine: bool, time: f32) {
    let (ca, cb) = (rgb(a), rgb(b));
    rrect(r, 12.0, cb);
    rrect(Rect::new(r.x + 1.5, r.y + 1.5, r.w - 3.0, r.h * 0.55), 11.0, ca);
    if shine {
        let x = r.x + ((time * 0.35).fract() * 1.6 - 0.3) * r.w;
        let w = r.w * 0.12;
        if x > r.x && x + w < r.x + r.w {
            draw_rectangle(x, r.y + 2.0, w, 2.5, Color::new(1.0, 1.0, 1.0, 0.7));
            draw_rectangle(x, r.y + r.h - 4.5, w, 2.5, Color::new(1.0, 1.0, 1.0, 0.5));
        }
    }
}

fn draw_styled_name_preview(ui: &Ui, label: &str, c: Vec2, style: Option<([u8; 3], [u8; 3])>) {
    let w = ui.text_size(label, 22.0, true).x;
    match style {
        Some((a, b)) => {
            ui.styled_text(label, c.x - w / 2.0, c.y - 11.0, 22.0, rgb(a), rgb(b), true)
        }
        None => {
            let color = crate::ui::tier_color(snookered_proto::Tier::Expert);
            ui.text(label, c.x - w / 2.0, c.y - 11.0, 22.0, color, true)
        }
    }
}

/// A small picture of an item.
fn preview(ui: &Ui, item: &Item, r: Rect) {
    let c = vec2(r.x + r.w / 2.0, r.y + r.h / 2.0);
    match item.look {
        Look::Cloth(col) => {
            // A corner of a table: rail, cushion and cloth.
            rrect(r, 8.0, Color::new(0.20, 0.09, 0.05, 1.0));
            let cloth = Rect::new(r.x + 10.0, r.y + 10.0, r.w - 20.0, r.h - 20.0);
            rrect(cloth, 5.0, rgb(col));
            let shade = Color::new(0.0, 0.0, 0.0, 0.18);
            draw_rectangle(cloth.x, cloth.y, cloth.w, 6.0, shade);
            draw_circle(c.x + cloth.w * 0.18, c.y + 4.0, 9.0, Color::new(0.96, 0.95, 0.9, 1.0));
            draw_circle(c.x - cloth.w * 0.2, c.y - 6.0, 9.0, Color::new(0.85, 0.12, 0.1, 1.0));
        }
        Look::Cue { butt, accent, ring, .. } => {
            let (x0, x1) = (r.x + 10.0, r.x + r.w - 10.0);
            let y = c.y;
            let lerp = |k: f32| x0 + (x1 - x0) * k;
            let part = |a: f32, b: f32, w0: f32, w1: f32, col: Color| {
                let (xa, xb) = (lerp(a), lerp(b));
                crate::paint::draw_triangle(
                    vec2(xa, y - w0),
                    vec2(xb, y - w1),
                    vec2(xb, y + w1),
                    col,
                );
                crate::paint::draw_triangle(
                    vec2(xa, y - w0),
                    vec2(xb, y + w1),
                    vec2(xa, y + w0),
                    col,
                );
            };
            part(0.0, 0.08, 6.0, 5.6, rgb(butt));
            part(0.08, 0.26, 5.6, 5.0, rgb(accent));
            part(0.26, 0.27, 5.0, 4.9, rgb(ring));
            part(0.27, 0.45, 4.9, 4.4, rgb(butt));
            part(0.45, 0.46, 4.4, 4.3, rgb(ring));
            part(0.46, 0.97, 4.3, 2.4, Color::new(0.87, 0.75, 0.53, 1.0));
            part(0.97, 0.99, 2.4, 2.3, Color::new(0.95, 0.94, 0.9, 1.0));
            part(0.99, 1.0, 2.3, 2.2, Color::new(0.16, 0.32, 0.6, 1.0));
        }
        Look::Chalk(col) => {
            let s = r.h.min(r.w) * 0.42;
            let top = Color::new(
                col[0] as f32 / 255.0 * 1.2,
                col[1] as f32 / 255.0 * 1.2,
                col[2] as f32 / 255.0 * 1.2,
                1.0,
            );
            draw_rectangle(c.x - s / 2.0, c.y - s / 2.0 + 6.0, s, s * 0.8, rgb(col));
            draw_rectangle(c.x - s / 2.0, c.y - s / 2.0, s, 8.0, top);
            draw_circle(c.x, c.y - s / 2.0 + 4.0, s * 0.22, Color::new(0.0, 0.0, 0.0, 0.25));
        }
        Look::Balls { colors, eight, ivory, .. } => {
            // Two rows: the solids and two stripes, and the 8.
            // Seven balls in a row must fit: 7 × 2.3 radii wide.
            let rad = (r.w / 17.0).min(r.h / 5.5);
            for (k, col) in colors.iter().enumerate() {
                let x = r.x + r.w / 2.0 + (k as f32 - 3.0) * rad * 2.3;
                draw_circle(x, c.y - rad * 1.2, rad, rgb(*col));
                draw_circle(x, c.y - rad * 1.2, rad * 0.42, rgb(ivory));
            }
            for k in 0..3 {
                let x = r.x + r.w / 2.0 + (k as f32 - 1.0) * rad * 2.3;
                let col = if k == 1 { eight } else { colors[k * 3] };
                draw_circle(x, c.y + rad * 1.4, rad, rgb(col));
                if k != 1 {
                    draw_rectangle(x - rad, c.y + rad * 0.9, rad * 2.0, rad, rgb(ivory));
                }
                draw_circle(x, c.y + rad * 1.4, rad * 0.42, rgb(ivory));
            }
        }
        Look::Frame { a, b, shine } => {
            // A name card inside the frame.
            rrect(r, 8.0, Color::new(0.0, 0.0, 0.0, 0.25));
            let card = Rect::new(c.x - 70.0, c.y - 30.0, 140.0, 60.0);
            frame(card, a, b, shine, get_time() as f32);
            rrect(
                Rect::new(card.x + 5.0, card.y + 5.0, card.w - 10.0, card.h - 10.0),
                8.0,
                Color::new(0.08, 0.05, 0.03, 1.0),
            );
            crate::icons::draw(Icon::Person, vec2(card.x + 26.0, c.y), 26.0, rgb(a));
        }
        Look::Name(style) => {
            rrect(r, 8.0, Color::new(0.0, 0.0, 0.0, 0.25));
            let label = crate::i18n::t("Your name");
            draw_styled_name_preview(ui, label, c, style);
        }
        Look::Effect { kind, color } => {
            rrect(r, 8.0, Color::new(0.03, 0.08, 0.05, 1.0));
            crate::effects::draw_preview(kind, color, r, get_time() as f32);
        }
    }
}

fn coin_amount(ui: &Ui, n: i64, x: f32, y: f32, size: f32, color: Color) -> f32 {
    crate::icons::draw(Icon::Coin, vec2(x + size * 0.55, y + size * 0.55), size, GOLD);
    let text = n.to_string();
    ui.text(&text, x + size * 1.3, y, size, color, true);
    size * 1.3 + ui.text_size(&text, size, true).x
}

/// The text shown for an item's kind and price on its card.
fn price_label(item: &Item, usable: bool) -> String {
    match item.price {
        Price::Free => t("free").to_string(),
        Price::Coins(_) if usable => t("yours").to_string(),
        Price::Coins(_) => t("for coins").to_string(),
        Price::Vip => "VIP".to_string(),
        Price::Achievement(_) => t("achievement").to_string(),
    }
}

impl ShopScreen {
    pub fn open(&mut self, online: &mut Online) {
        self.confirm = None;
        self.gift_pick = None;
        self.done = None;
        online.bought = None;
        online.shop_error = None;
        self.asked = online.signed_in();
        if online.signed_in() {
            online.send(ClientMsg::Shop);
            online.send(ClientMsg::Friends);
        }
    }

    /// Opens on the tab of an item (back from the 3D preview).
    pub fn show_item(&mut self, code: &str) {
        if let Some(item) = snookered_proto::shop::item(code) {
            self.tab = Tab::Items(item.category);
            self.first = 0;
        }
    }

    /// Asks to buy `code` (the confirmation dialog), e.g. from the 3D preview.
    pub fn ask_to_buy(&mut self, code: &'static str) {
        self.confirm = Some(Confirm { code, gift_to: None });
    }

    /// Development aid (`--screen shop-coins`…): opens on a tab.
    pub fn dev_tab(&mut self, name: &str) {
        self.tab = match name {
            "shop-coins" => Tab::Coins,
            "shop-vip" => Tab::Vip,
            "shop-balls" => Tab::Items(Category::Balls),
            "shop-cues" => Tab::Items(Category::Cue),
            "shop-chalk" => Tab::Items(Category::Chalk),
            "shop-frames" => Tab::Items(Category::Frame),
            "shop-names" => Tab::Items(Category::Name),
            "shop-effects" => Tab::Items(Category::Effect),
            _ => Tab::Items(Category::Cloth),
        };
        if name == "shop-confirm" {
            self.confirm = Some(Confirm { code: "cloth_purple", gift_to: None });
        }
        if name == "shop-done" {
            self.done = Some(("balls_neon", None));
        }
    }

    pub fn draw(&mut self, ui: &mut Ui, online: &mut Online) -> ShopAction {
        // Signed in after the screen opened: ask for the wallet now.
        if online.signed_in() && (online.wallet.is_none() || online.offers.is_none()) && !self.asked
        {
            self.asked = true;
            online.send(ClientMsg::Shop);
            online.send(ClientMsg::Friends);
        }
        // The server confirmed a purchase: show it.
        if let Some((code, gift)) = online.bought.take()
            && let Some(item) = snookered_proto::shop::item(&code)
        {
            self.confirm = None;
            self.done = Some((item.code, gift));
        }
        let modal = self.confirm.is_some() || self.gift_pick.is_some() || self.done.is_some();
        let saved = modal.then(|| ui.suspend_input());
        let mut action = self.page(ui, online);
        if let Some(saved) = saved {
            ui.resume_input(saved);
            draw_rectangle(
                0.0,
                0.0,
                crate::ui::vw(),
                crate::ui::vh(),
                Color::new(0.0, 0.0, 0.0, 0.55),
            );
            if self.done.is_some() {
                self.done_dialog(ui);
            } else if self.gift_pick.is_some() {
                self.gift_dialog(ui, online);
            } else if self.confirm.is_some()
                && let Some(a) = self.confirm_dialog(ui, online)
            {
                action = a;
            }
        }
        action
    }

    fn page(&mut self, ui: &mut Ui, online: &mut Online) -> ShopAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        let w = (sw - 40.0).min(1060.0);
        let h = (sh - 40.0).min(700.0);
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
        ui.panel(r);
        let mut action = ShopAction::None;
        if ui.icon_button(
            Rect::new(r.x + 14.0, r.y + 14.0, 34.0, 34.0),
            Icon::Back,
            t("Back"),
            true,
        ) || (is_key_pressed(KeyCode::Escape) && ui.focus.is_none())
        {
            action = ShopAction::Back;
        }
        crate::icons::draw(Icon::Shop, vec2(r.x + 70.0, r.y + 31.0), 22.0, GOLD);
        ui.text(t("Shop"), r.x + 90.0, r.y + 17.0, 24.0, GOLD, true);
        let signed_in = online.signed_in();
        let wallet = online.wallet.clone().unwrap_or_default();
        // Balance and VIP, top right.
        if signed_in {
            let mut x = r.x + r.w - 20.0;
            let coins = wallet.coins.to_string();
            let cw = ui.text_size(&coins, 18.0, true).x + 30.0;
            x -= cw;
            coin_amount(
                ui,
                wallet.coins,
                x,
                r.y + 20.0,
                18.0,
                if wallet.coins < 0 { ALERT } else { CREAM },
            );
            if wallet.is_vip() {
                x -= 70.0;
                crate::icons::draw(Icon::Crown, vec2(x + 10.0, r.y + 30.0), 18.0, GOLD);
                ui.text("VIP", x + 24.0, r.y + 21.0, 16.0, GOLD, true);
            }
        }
        // The categories, down the left side.
        let side = Rect::new(r.x + 16.0, r.y + 62.0, 180.0, r.h - 78.0);
        let mut y = side.y;
        for c in Category::ALL {
            let tab = Tab::Items(c);
            let owned = ITEMS
                .iter()
                .filter(|i| i.category == c && (i.price == Price::Free || online.can_use(i)))
                .count();
            let total = ITEMS.iter().filter(|i| i.category == c).count();
            let tr = Rect::new(side.x, y, side.w, 34.0);
            let count = if signed_in { format!("{owned}/{total}") } else { String::new() };
            if ui.tab_with_count(tr, category_name(c), &count, self.tab == tab) {
                self.tab = tab;
                self.first = 0;
            }
            y += 38.0;
        }
        y += 10.0;
        draw_line(side.x + 8.0, y - 5.0, side.x + side.w - 8.0, y - 5.0, 1.0, BORDER);
        for (tab, label) in [(Tab::Coins, t("Coins")), (Tab::Vip, "VIP")] {
            if ui.tab(Rect::new(side.x, y, side.w, 34.0), label, self.tab == tab) {
                self.tab = tab;
                self.first = 0;
            }
            y += 38.0;
        }
        let body = Rect::new(
            side.x + side.w + 16.0,
            r.y + 64.0,
            r.x + r.w - (side.x + side.w + 16.0) - 18.0,
            r.h - 82.0,
        );
        if !signed_in {
            let note = t("Sign in to buy and use items. Here is what there is:");
            ui.text(note, body.x, body.y, 14.0, MUTED, false);
        }
        let body =
            if signed_in { body } else { Rect::new(body.x, body.y + 26.0, body.w, body.h - 26.0) };
        match self.tab {
            Tab::Items(c) => {
                if let Some(a) = self.items(ui, online, c, body) {
                    action = a;
                }
            }
            Tab::Coins => self.coins(ui, online, body),
            Tab::Vip => self.vip(ui, online, body),
        }
        action
    }

    fn items(
        &mut self,
        ui: &mut Ui,
        online: &mut Online,
        category: Category,
        area: Rect,
    ) -> Option<ShopAction> {
        let items: Vec<&'static Item> = ITEMS.iter().filter(|i| i.category == category).collect();
        // Room for the scroll bar on the right.
        let area = Rect::new(area.x, area.y, area.w - 14.0, area.h);
        let signed_in = online.signed_in();
        let wallet = online.wallet.clone().unwrap_or_default();
        let in_use = online.effective(&wallet, category);
        let cols = ((area.w + 12.0) / 206.0).floor().max(1.0) as usize;
        let cw = (area.w - (cols - 1) as f32 * 12.0) / cols as f32;
        let ch = 206.0;
        let rows_fit = ((area.h + 12.0) / (ch + 12.0)).floor().max(1.0) as usize;
        let total_rows = items.len().div_ceil(cols);
        self.first = ui.scroll_list(area, self.first, total_rows, rows_fit);
        let mut tip = None;
        let mut action = None;
        for (k, item) in items.iter().enumerate().skip(self.first * cols).take(rows_fit * cols) {
            let k = k - self.first * cols;
            let c = Rect::new(
                area.x + (k % cols) as f32 * (cw + 12.0),
                area.y + (k / cols) as f32 * (ch + 12.0),
                cw,
                ch,
            );
            let usable = signed_in && online.can_use(item) || item.price == Price::Free;
            let equipped = signed_in && in_use == item.code;
            rrect(c, 10.0, Color::new(1.0, 1.0, 1.0, if equipped { 0.08 } else { 0.035 }));
            rrect_lines(
                c,
                10.0,
                if equipped { 2.0 } else { 1.0 },
                if equipped { GOLD } else { BORDER },
            );
            let pic = Rect::new(c.x + 12.0, c.y + 12.0, c.w - 24.0, 96.0);
            preview(ui, item, pic);
            // Try it on the table, or give it to a friend.
            let mut bx = pic.x + pic.w - 30.0;
            if has_3d_preview(item) {
                if ui.icon_button(
                    Rect::new(bx, pic.y + 4.0, 26.0, 26.0),
                    Icon::Eye,
                    t("Try it on the table"),
                    true,
                ) {
                    action = Some(ShopAction::Preview(item.code));
                }
                bx -= 30.0;
            }
            if signed_in
                && item.coins().is_some()
                && !online.lan
                && ui.icon_button(
                    Rect::new(bx, pic.y + 4.0, 26.0, 26.0),
                    Icon::Gift,
                    t("Give to a friend"),
                    true,
                )
            {
                self.gift_pick = Some(item.code);
                online.send(ClientMsg::Friends);
            }
            let (name, size) = ui.fitted(t(item.name), 15.0, c.w - 24.0, true);
            ui.text(&name, c.x + 12.0, c.y + 116.0, size, CREAM, true);
            ui.text(&price_label(item, usable), c.x + 12.0, c.y + 137.0, 12.0, MUTED, false);
            let b = Rect::new(c.x + 12.0, c.y + c.h - 46.0, c.w - 24.0, 34.0);
            if equipped {
                ui.button(b, t("In use"), false);
            } else if usable {
                if ui.button(b, t("Use"), signed_in) {
                    online.send(ClientMsg::Equip { code: item.code.to_string() });
                }
            } else {
                match item.price {
                    Price::Coins(n) => {
                        let afford = wallet.coins >= n as i64;
                        let label = tw("Buy · {n}", &[("n", &n)]);
                        let hit = if signed_in && afford {
                            ui.primary_button(b, &label)
                        } else {
                            ui.button(b, &label, signed_in)
                        };
                        if hit {
                            self.confirm = Some(Confirm { code: item.code, gift_to: None });
                            online.shop_error = None;
                        }
                        if ui.text_size(&label, 15.0, true).x + 48.0 < b.w {
                            crate::icons::draw(
                                Icon::Coin,
                                vec2(b.x + 16.0, b.y + b.h / 2.0),
                                14.0,
                                GOLD,
                            );
                        }
                        if signed_in && !afford && ui.hovering(b) {
                            tip = Some((
                                t("Not enough coins: get more in the Coins tab.").to_string(),
                                b,
                            ));
                        }
                    }
                    Price::Vip => {
                        if ui.button(b, t("VIP only"), true) {
                            self.tab = Tab::Vip;
                        }
                    }
                    Price::Achievement(a) => {
                        let how = snookered_proto::ACHIEVEMENTS
                            .iter()
                            .find(|x| x.code == a)
                            .map(|x| t(x.title))
                            .unwrap_or(a);
                        ui.button(b, &tw("Achievement: {a}", &[("a", &how)]), false);
                        if ui.hovering(b) {
                            let desc = snookered_proto::ACHIEVEMENTS
                                .iter()
                                .find(|x| x.code == a)
                                .map(|x| t(x.description).to_string())
                                .unwrap_or_default();
                            tip = Some((desc, b));
                        }
                    }
                    Price::Free => {}
                }
            }
        }
        if let Some((text, r)) = tip {
            ui.tooltip(&text, r);
        }
        action
    }

    /// "Buy this?": the item, what it costs, the coins before and after.
    fn confirm_dialog(&mut self, ui: &mut Ui, online: &mut Online) -> Option<ShopAction> {
        let confirm = self.confirm.clone()?;
        let item = snookered_proto::shop::item(confirm.code)?;
        let price = item.coins().unwrap_or(0) as i64;
        let coins = online.wallet.as_ref().map(|w| w.coins).unwrap_or(0);
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let w = 480.0f32.min(sw - 40.0);
        let h = 430.0;
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
        ui.panel(r);
        let gift = confirm.gift_to.is_some();
        let title = if gift { t("Give as a gift") } else { t("Confirm purchase") };
        ui.text_centered(title, r.x + w / 2.0, r.y + 18.0, 22.0, GOLD, true);
        let pic = Rect::new(r.x + 40.0, r.y + 58.0, w - 80.0, 120.0);
        preview(ui, item, pic);
        let mut y = pic.y + pic.h + 14.0;
        ui.text_centered(t(item.name), r.x + w / 2.0, y, 20.0, CREAM, true);
        y += 26.0;
        ui.text_centered(category_name(item.category), r.x + w / 2.0, y, 13.0, MUTED, false);
        y += 28.0;
        if let Some(friend) = &confirm.gift_to {
            ui.text_centered(
                &tw("For {name}", &[("name", friend)]),
                r.x + w / 2.0,
                y,
                15.0,
                CREAM,
                true,
            );
            y += 26.0;
        }
        // The price, and the coins before and after, in one clear line.
        let after = coins - price;
        let line_w = 300.0;
        let lx = r.x + (w - line_w) / 2.0;
        ui.text(t("Price"), lx, y, 15.0, MUTED, false);
        coin_amount(ui, price, lx + 170.0, y - 2.0, 17.0, CREAM);
        y += 26.0;
        ui.text(t("Your coins"), lx, y, 15.0, MUTED, false);
        crate::icons::draw(Icon::Coin, vec2(lx + 170.0 + 9.0, y + 9.0), 17.0, GOLD);
        if after < 0 {
            ui.text(&coins.to_string(), lx + 192.0, y - 1.0, 17.0, CREAM, true);
            y += 26.0;
            let short = tw("You need {n} more coins.", &[("n", &-after)]);
            ui.text_centered(&short, r.x + w / 2.0, y, 15.0, ALERT, true);
        } else {
            let text = format!("{coins}  →  {after}");
            ui.text(&text, lx + 192.0, y - 1.0, 17.0, CREAM, true);
        }
        y += 32.0;
        let note = if gift {
            t("Your friend gets it right away.")
        } else {
            t("It goes on right away. Items only change the look, never the game.")
        };
        for (i, line) in ui.wrap(note, w - 60.0, 13.0).iter().enumerate() {
            ui.text_centered(line, r.x + w / 2.0, y + i as f32 * 17.0, 13.0, MUTED, false);
        }
        if let Some(e) = &online.shop_error {
            ui.text_centered(e, r.x + w / 2.0, r.y + h - 82.0, 14.0, ALERT, true);
        }
        let by = r.y + h - 56.0;
        let half = (w - 60.0) / 2.0;
        let cancel =
            ui.button(Rect::new(r.x + 20.0, by, half, 40.0), t("Cancel"), online.buying.is_none());
        let ok_rect = Rect::new(r.x + 40.0 + half, by, half, 40.0);
        let mut action = None;
        if online.buying.is_some() {
            ui.button(ok_rect, t("Buying…"), false);
        } else if after < 0 {
            if ui.primary_button(ok_rect, t("Get coins")) {
                self.confirm = None;
                self.tab = Tab::Coins;
            }
        } else {
            let label = if gift { t("Give it") } else { t("Buy now") };
            if ui.primary_button(ok_rect, label) || is_key_pressed(KeyCode::Enter) {
                online.shop_error = None;
                online.buying = Some(item.code.to_string());
                match &confirm.gift_to {
                    Some(to) => online
                        .send(ClientMsg::GiftItem { to: to.clone(), code: item.code.to_string() }),
                    None => online.send(ClientMsg::BuyItem { code: item.code.to_string() }),
                }
            }
        }
        if !gift && has_3d_preview(item) && online.buying.is_none() {
            let pr = Rect::new(pic.x + pic.w - 30.0, pic.y + 4.0, 26.0, 26.0);
            if ui.icon_button(pr, Icon::Eye, t("Try it on the table"), true) {
                self.confirm = None;
                action = Some(ShopAction::Preview(item.code));
            }
        }
        if (cancel || is_key_pressed(KeyCode::Escape)) && online.buying.is_none() {
            self.confirm = None;
            online.shop_error = None;
        }
        action
    }

    /// The bar under the 3D preview: the item, its price, buy or go back.
    pub fn preview_bar(
        &mut self,
        ui: &mut Ui,
        online: &mut Online,
        code: &'static str,
    ) -> PreviewAction {
        let Some(item) = snookered_proto::shop::item(code) else { return PreviewAction::Close };
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let w = 620.0f32.min(sw - 30.0);
        let r = Rect::new((sw - w) / 2.0, sh - 104.0, w, 84.0);
        ui.panel(r);
        ui.text(t(item.name), r.x + 20.0, r.y + 14.0, 20.0, GOLD, true);
        let sub = format!("{} · {}", category_name(item.category), t("turning around the table"));
        ui.text(&sub, r.x + 20.0, r.y + 44.0, 13.0, MUTED, false);
        let usable = online.signed_in() && online.can_use(item) || item.price == Price::Free;
        let mut action = PreviewAction::None;
        let b = Rect::new(r.x + r.w - 150.0, r.y + 22.0, 130.0, 40.0);
        match item.price {
            Price::Coins(n) if !usable && online.signed_in() => {
                if ui.primary_button(b, &tw("Buy · {n}", &[("n", &n)])) {
                    action = PreviewAction::Buy;
                }
            }
            _ if usable && online.signed_in() => {
                let wallet = online.wallet.clone().unwrap_or_default();
                let in_use = online.effective(&wallet, item.category) == item.code;
                if in_use {
                    ui.button(b, t("In use"), false);
                } else if ui.primary_button(b, t("Use")) {
                    online.send(ClientMsg::Equip { code: item.code.to_string() });
                }
            }
            _ => {}
        }
        let back = Rect::new(b.x - 150.0, b.y, 140.0, 40.0);
        if ui.button(back, t("Back to the shop"), true) || is_key_pressed(KeyCode::Escape) {
            action = PreviewAction::Close;
        }
        action
    }

    /// The purchase went through.
    fn done_dialog(&mut self, ui: &mut Ui) {
        let Some((code, gift)) = self.done.clone() else { return };
        let Some(item) = snookered_proto::shop::item(code) else {
            self.done = None;
            return;
        };
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let w = 440.0f32.min(sw - 40.0);
        let h = 370.0;
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
        ui.panel(r);
        draw_circle(r.x + w / 2.0, r.y + 46.0, 26.0, GOLD);
        crate::icons::draw(
            Icon::Check,
            vec2(r.x + w / 2.0, r.y + 46.0),
            28.0,
            Color::new(0.1, 0.07, 0.04, 1.0),
        );
        let title = if gift.is_some() { t("Gift sent!") } else { t("Purchase complete!") };
        ui.text_centered(title, r.x + w / 2.0, r.y + 84.0, 22.0, GOLD, true);
        let pic = Rect::new(r.x + 50.0, r.y + 120.0, w - 100.0, 110.0);
        preview(ui, item, pic);
        let text = match &gift {
            Some(friend) => {
                tw("{name} received {item}.", &[("name", friend), ("item", &t(item.name))])
            }
            None => tw("{item} is yours and already in use.", &[("item", &t(item.name))]),
        };
        for (i, line) in ui.wrap(&text, w - 50.0, 15.0).iter().enumerate() {
            ui.text_centered(
                line,
                r.x + w / 2.0,
                r.y + 244.0 + i as f32 * 20.0,
                15.0,
                CREAM,
                false,
            );
        }
        let b = Rect::new(r.x + (w - 200.0) / 2.0, r.y + h - 58.0, 200.0, 40.0);
        if ui.primary_button(b, t("Great!"))
            || is_key_pressed(KeyCode::Escape)
            || is_key_pressed(KeyCode::Enter)
        {
            self.done = None;
        }
    }

    /// Choosing which friend gets a present.
    fn gift_dialog(&mut self, ui: &mut Ui, online: &mut Online) {
        let Some(code) = self.gift_pick else { return };
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let w = 420.0f32.min(sw - 40.0);
        let h = 440.0f32.min(sh - 40.0);
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
        ui.panel(r);
        crate::icons::draw(Icon::Gift, vec2(r.x + 30.0, r.y + 30.0), 22.0, GOLD);
        ui.text(t("Give to a friend"), r.x + 50.0, r.y + 18.0, 20.0, GOLD, true);
        if let Some(item) = snookered_proto::shop::item(code) {
            ui.text(t(item.name), r.x + 50.0, r.y + 44.0, 14.0, MUTED, false);
        }
        let friends = online.friends.friends.clone();
        let list = Rect::new(r.x + 16.0, r.y + 74.0, w - 32.0, h - 140.0);
        if friends.is_empty() {
            for (i, line) in
                ui.wrap(t("Add friends to give them items."), list.w, 14.0).iter().enumerate()
            {
                ui.text(line, list.x, list.y + i as f32 * 18.0, 14.0, MUTED, false);
            }
        }
        let row_h = 40.0;
        for (k, f) in friends.iter().enumerate().take((list.h / row_h) as usize) {
            let row = Rect::new(list.x, list.y + k as f32 * row_h, list.w, row_h - 4.0);
            let hover = ui.hovering(row);
            rrect(row, 6.0, Color::new(1.0, 1.0, 1.0, if hover { 0.09 } else { 0.04 }));
            ui.player_name(&f.player, row.x + 10.0, row.y + 10.0, 15.0, row.w - 20.0);
            ui.block(row);
            if hover && ui.clicked(row) {
                self.gift_pick = None;
                self.confirm = Some(Confirm { code, gift_to: Some(f.player.username.clone()) });
                online.shop_error = None;
            }
        }
        let b = Rect::new(r.x + (w - 160.0) / 2.0, r.y + h - 54.0, 160.0, 38.0);
        if ui.button(b, t("Cancel"), true) || is_key_pressed(KeyCode::Escape) {
            self.gift_pick = None;
        }
    }

    fn coins(&mut self, ui: &mut Ui, online: &mut Online, area: Rect) {
        let offers = online.offers.clone().unwrap_or_default();
        if !crate::channel::real_money_purchases() {
            for (i, line) in [
                t("In this version of the game, coins are earned by playing:"),
                t("rated games, and a bonus every day you play."),
            ]
            .iter()
            .enumerate()
            {
                ui.text(line, area.x, area.y + 6.0 + i as f32 * 22.0, 15.0, CREAM, false);
            }
            return;
        }
        if !offers.payments || offers.packs.is_empty() {
            ui.text(
                t("Coins can't be bought on this server yet."),
                area.x,
                area.y + 6.0,
                15.0,
                MUTED,
                false,
            );
            return;
        }
        let signed_in = online.signed_in();
        let n = offers.packs.len();
        let cols = n.min(((area.w + 12.0) / 200.0).floor().max(1.0) as usize);
        let cw = (area.w - (cols - 1) as f32 * 12.0) / cols as f32;
        for (k, p) in offers.packs.iter().enumerate() {
            let c = Rect::new(
                area.x + (k % cols) as f32 * (cw + 12.0),
                area.y + (k / cols) as f32 * 190.0,
                cw,
                176.0,
            );
            rrect(c, 10.0, Color::new(1.0, 1.0, 1.0, 0.04));
            rrect_lines(c, 10.0, 1.0, BORDER);
            // A small pile of coins, bigger for bigger packs.
            for i in 0..(k + 2).min(5) {
                crate::icons::draw(
                    Icon::Coin,
                    vec2(c.x + c.w / 2.0 - 20.0 + i as f32 * 10.0, c.y + 34.0 - i as f32 * 3.0),
                    26.0,
                    GOLD,
                );
            }
            let total = (p.coins + p.bonus).to_string();
            ui.text_centered(&total, c.x + c.w / 2.0, c.y + 62.0, 24.0, CREAM, true);
            ui.text_centered(t("coins"), c.x + c.w / 2.0, c.y + 90.0, 13.0, MUTED, false);
            if p.bonus > 0 {
                ui.text_centered(
                    &tw("+{n} bonus", &[("n", &p.bonus)]),
                    c.x + c.w / 2.0,
                    c.y + 108.0,
                    13.0,
                    GOLD,
                    true,
                );
            }
            let b = Rect::new(c.x + 12.0, c.y + c.h - 44.0, c.w - 24.0, 34.0);
            if signed_in {
                if ui.primary_button(b, &p.price) {
                    online.send(ClientMsg::BuyCoins { pack: p.code.clone() });
                }
            } else {
                ui.button(b, &p.price, false);
            }
        }
        let rows = n.div_ceil(cols) as f32;
        let y = area.y + rows * 190.0 + 4.0;
        for (i, line) in [
            t("Payment opens in your browser (card, Pix…), on Stripe's secure page."),
            t("Coins arrive in a moment; they are only used for items in the game."),
        ]
        .iter()
        .enumerate()
        {
            ui.text(line, area.x, y + i as f32 * 20.0, 13.0, MUTED, false);
        }
    }

    fn vip(&mut self, ui: &mut Ui, online: &mut Online, area: Rect) {
        let offers = online.offers.clone().unwrap_or_default();
        let wallet = online.wallet.clone().unwrap_or_default();
        crate::icons::draw(Icon::Crown, vec2(area.x + 22.0, area.y + 18.0), 30.0, GOLD);
        ui.text(
            &format!("{} VIP", snookered_proto::GAME_NAME),
            area.x + 48.0,
            area.y + 4.0,
            24.0,
            GOLD,
            true,
        );
        let mut y = area.y + 50.0;
        let exclusives: Vec<String> =
            ITEMS.iter().filter(|i| i.price == Price::Vip).map(|i| t(i.name).to_string()).collect();
        let perks = [
            tw("{n} coins every month", &[("n", &offers.vip_coins)]),
            t("A crown by your name, everywhere (lounge, profile, ranking)").to_string(),
            t("Name colours for the lounge and exclusive frames, cloth, cues, balls and effects")
                .to_string(),
            tw("Exclusive items: {list}", &[("list", &exclusives.join(", "))]),
            t("Every replay kept (free players keep their last 10 games)").to_string(),
            t("16-player tournaments, and first place on the waiting list of a full one")
                .to_string(),
            t("Nothing that wins games: VIP only changes the look").to_string(),
            t("Cancel whenever you want: VIP lasts until the end of the paid month").to_string(),
        ];
        for p in &perks {
            crate::icons::draw(Icon::Star, vec2(area.x + 10.0, y + 9.0), 12.0, GOLD);
            let lines = ui.wrap(p, area.w - 40.0, 15.0);
            for (i, line) in lines.iter().enumerate() {
                ui.text(line, area.x + 26.0, y + i as f32 * 19.0, 15.0, CREAM, false);
            }
            y += lines.len().saturating_sub(1) as f32 * 19.0 + 24.0;
        }
        y += 6.0;
        let b = Rect::new(area.x, y, 280.0f32.min(area.w), 42.0);
        if wallet.is_vip() {
            let until = crate::profile::date(wallet.vip_until);
            ui.text(
                &tw("You are VIP until {d}.", &[("d", &until)]),
                area.x,
                y - 4.0,
                15.0,
                GOLD,
                true,
            );
            // The subscription is managed on the payment provider's pages.
            if crate::channel::real_money_purchases() {
                let b = Rect::new(area.x, y + 24.0, 280.0f32.min(area.w), 38.0);
                if ui.button(b, t("Manage subscription"), offers.payments) {
                    online.send(ClientMsg::ManageVip);
                }
            }
        } else if !crate::channel::real_money_purchases() {
            ui.text(
                t("VIP isn't sold in this version of the game."),
                area.x,
                y,
                15.0,
                MUTED,
                false,
            );
        } else if offers.payments && !offers.vip_price.is_empty() {
            if ui.primary_button(b, &tw("Become VIP · {p}", &[("p", &offers.vip_price)])) {
                online.send(ClientMsg::SubscribeVip);
            }
        } else {
            ui.text(t("VIP can't be bought on this server yet."), area.x, y, 15.0, MUTED, false);
        }
    }
}
