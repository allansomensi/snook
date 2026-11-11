//! The shop: cloth, cues, chalk and ball sets (for coins, with achievements
//! or for VIP members), coin packs and the VIP subscription.
//!
//! The server is the authority: this screen only asks it to buy, use or
//! open a payment page, and shows what it answers.

use macroquad::prelude::*;
use snook_proto::ClientMsg;
use snook_proto::shop::{Category, ITEMS, Item, Look, Price};

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

pub enum ShopAction {
    None,
    Back,
}

pub struct ShopScreen {
    tab: Tab,
    /// The item waiting for "click again to buy".
    confirm: Option<&'static str>,
    first: usize,
    asked: bool,
}

impl Default for ShopScreen {
    fn default() -> Self {
        ShopScreen { tab: Tab::Items(Category::Cloth), confirm: None, first: 0, asked: false }
    }
}

fn rgb(c: [u8; 3]) -> Color {
    Color::from_rgba(c[0], c[1], c[2], 255)
}

fn category_name(c: Category) -> &'static str {
    match c {
        Category::Cloth => t("Cloth"),
        Category::Cue => t("Cues"),
        Category::Chalk => t("Chalk"),
        Category::Balls => t("Balls"),
    }
}

/// A small picture of an item.
fn preview(item: &Item, r: Rect) {
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
        Look::Cue { butt, accent, ring } => {
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
        Look::Balls { colors, eight, ivory } => {
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
    }
}

fn coin_amount(ui: &Ui, n: i64, x: f32, y: f32, size: f32, color: Color) -> f32 {
    crate::icons::draw(Icon::Coin, vec2(x + size * 0.55, y + size * 0.55), size, GOLD);
    let text = n.to_string();
    ui.text(&text, x + size * 1.3, y, size, color, true);
    size * 1.3 + ui.text_size(&text, size, true).x
}

impl ShopScreen {
    pub fn open(&mut self, online: &mut Online) {
        self.confirm = None;
        self.asked = online.signed_in();
        if online.signed_in() {
            online.send(ClientMsg::Shop);
        }
    }

    /// Development aid (`--screen shop-coins`…): opens on a tab.
    pub fn dev_tab(&mut self, name: &str) {
        self.tab = match name {
            "shop-coins" => Tab::Coins,
            "shop-vip" => Tab::Vip,
            "shop-balls" => Tab::Items(Category::Balls),
            "shop-cues" => Tab::Items(Category::Cue),
            "shop-chalk" => Tab::Items(Category::Chalk),
            _ => Tab::Items(Category::Cloth),
        };
    }

    pub fn draw(&mut self, ui: &mut Ui, online: &mut Online) -> ShopAction {
        // Signed in after the screen opened: ask for the wallet now.
        if online.signed_in() && (online.wallet.is_none() || online.offers.is_none()) && !self.asked
        {
            self.asked = true;
            online.send(ClientMsg::Shop);
        }
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        let w = (sw - 40.0).min(980.0);
        let h = (sh - 40.0).min(680.0);
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
        // Tabs.
        let tabs = [
            (Tab::Items(Category::Cloth), category_name(Category::Cloth)),
            (Tab::Items(Category::Cue), category_name(Category::Cue)),
            (Tab::Items(Category::Chalk), category_name(Category::Chalk)),
            (Tab::Items(Category::Balls), category_name(Category::Balls)),
            (Tab::Coins, t("Coins")),
            (Tab::Vip, "VIP"),
        ];
        let tw_ = (r.w - 40.0 - 5.0 * 8.0) / tabs.len() as f32;
        for (k, (tab, label)) in tabs.iter().enumerate() {
            let tr = Rect::new(r.x + 20.0 + k as f32 * (tw_ + 8.0), r.y + 62.0, tw_, 32.0);
            if ui.tab(tr, label, self.tab == *tab) {
                self.tab = *tab;
                self.confirm = None;
                self.first = 0;
            }
        }
        let body = Rect::new(r.x + 20.0, r.y + 108.0, r.w - 40.0, r.h - 128.0);
        if !signed_in {
            let note = t("Sign in to buy and use items. Here is what there is:");
            ui.text(note, body.x, body.y, 14.0, MUTED, false);
        }
        let body =
            if signed_in { body } else { Rect::new(body.x, body.y + 26.0, body.w, body.h - 26.0) };
        match self.tab {
            Tab::Items(c) => self.items(ui, online, c, body),
            Tab::Coins => self.coins(ui, online, body),
            Tab::Vip => self.vip(ui, online, body),
        }
        action
    }

    fn items(&mut self, ui: &mut Ui, online: &mut Online, category: Category, area: Rect) {
        let items: Vec<&Item> = ITEMS.iter().filter(|i| i.category == category).collect();
        // Room for the scroll bar on the right.
        let area = Rect::new(area.x, area.y, area.w - 14.0, area.h);
        let signed_in = online.signed_in();
        let wallet = online.wallet.clone().unwrap_or_default();
        let in_use = online.effective(&wallet, category);
        let cols = ((area.w + 12.0) / 222.0).floor().max(1.0) as usize;
        let cw = (area.w - (cols - 1) as f32 * 12.0) / cols as f32;
        let ch = 196.0;
        let rows_fit = ((area.h + 12.0) / (ch + 12.0)).floor().max(1.0) as usize;
        let total_rows = items.len().div_ceil(cols);
        self.first = ui.scroll_list(area, self.first, total_rows, rows_fit);
        let mut tip = None;
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
            preview(item, Rect::new(c.x + 12.0, c.y + 12.0, c.w - 24.0, 96.0));
            let (name, size) = ui.fitted(t(item.name), 15.0, c.w - 24.0, true);
            ui.text(&name, c.x + 12.0, c.y + 116.0, size, CREAM, true);
            let b = Rect::new(c.x + 12.0, c.y + c.h - 44.0, c.w - 24.0, 32.0);
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
                        let label = if self.confirm == Some(item.code) {
                            tw("Buy for {n}?", &[("n", &n)])
                        } else {
                            tw("{n} coins", &[("n", &n)])
                        };
                        if ui.button(b, &label, signed_in && afford) {
                            if self.confirm == Some(item.code) {
                                online.send(ClientMsg::BuyItem { code: item.code.to_string() });
                                self.confirm = None;
                            } else {
                                self.confirm = Some(item.code);
                            }
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
                        let how = snook_proto::ACHIEVEMENTS
                            .iter()
                            .find(|x| x.code == a)
                            .map(|x| t(x.title))
                            .unwrap_or(a);
                        ui.button(b, &tw("Achievement: {a}", &[("a", &how)]), false);
                        if ui.hovering(b) {
                            let desc = snook_proto::ACHIEVEMENTS
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
            let kind = match item.price {
                Price::Free => t("free"),
                Price::Coins(_) if usable => t("yours"),
                Price::Coins(_) => t("for coins"),
                Price::Vip => "VIP",
                Price::Achievement(_) => t("achievement"),
            };
            ui.text(kind, c.x + 12.0, c.y + 136.0, 12.0, MUTED, false);
        }
        if let Some((text, r)) = tip {
            ui.tooltip(&text, r);
        }
    }

    fn coins(&mut self, ui: &mut Ui, online: &mut Online, area: Rect) {
        let offers = online.offers.clone().unwrap_or_default();
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
        ui.text("Snook VIP", area.x + 48.0, area.y + 4.0, 24.0, GOLD, true);
        let mut y = area.y + 50.0;
        let exclusives: Vec<String> =
            ITEMS.iter().filter(|i| i.price == Price::Vip).map(|i| t(i.name).to_string()).collect();
        let perks = [
            tw("Exclusive items: {list}", &[("list", &exclusives.join(", "))]),
            tw("{n} coins every month", &[("n", &offers.vip_coins)]),
            t("A crown by your name, everywhere").to_string(),
            t("Cancel whenever you want: VIP lasts until the end of the paid month").to_string(),
        ];
        for p in &perks {
            crate::icons::draw(Icon::Star, vec2(area.x + 10.0, y + 9.0), 12.0, GOLD);
            for (i, line) in ui.wrap(p, area.w - 40.0, 15.0).iter().enumerate() {
                ui.text(line, area.x + 26.0, y + i as f32 * 19.0, 15.0, CREAM, false);
                if i > 0 {
                    y += 19.0;
                }
            }
            y += 30.0;
        }
        y += 10.0;
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
            let b = Rect::new(area.x, y + 24.0, 280.0f32.min(area.w), 38.0);
            if ui.button(b, t("Manage subscription"), offers.payments) {
                online.send(ClientMsg::ManageVip);
            }
        } else if offers.payments && !offers.vip_price.is_empty() {
            if ui.primary_button(b, &tw("Become VIP · {p}", &[("p", &offers.vip_price)])) {
                online.send(ClientMsg::SubscribeVip);
            }
        } else {
            ui.text(t("VIP can't be bought on this server yet."), area.x, y, 15.0, MUTED, false);
        }
    }
}
