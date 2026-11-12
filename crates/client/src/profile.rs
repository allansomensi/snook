//! A player's profile: rating, numbers, achievements and recent games, with
//! the friend, block and report actions.

use macroquad::prelude::*;
use snook_proto::{ACHIEVEMENTS, ClientMsg, Profile, Relation, ReportReason};

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_line, draw_rectangle};

use crate::i18n::{t, tw};
use crate::icons::Icon;
use crate::online::Online;
use crate::ui::{ALERT, BORDER, CREAM, GOLD, MUTED, Ui, rrect, rrect_lines, tier_color};

pub enum ProfileAction {
    None,
    Back,
}

/// What the right-hand side of the profile shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Pane {
    #[default]
    Games,
    Rating,
    Numbers,
}

#[derive(Default)]
pub struct ProfileScreen {
    pub name: String,
    history_first: usize,
    report: Option<(usize, String)>,
    confirm_block: bool,
    pane: Pane,
    /// The account dialog (own profile).
    account: Option<AccountForm>,
}

/// The account dialog: recovery e-mail, password, deletion.
#[derive(Default)]
struct AccountForm {
    email: String,
    email_loaded: bool,
    old_password: String,
    new_password: String,
    delete_password: String,
    confirm_delete: bool,
}

/// "3 days", "2 months", "1 year".
fn age(created_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = ((now - created_at).max(0) / 86_400) as u32;
    if days < 1 {
        t("joined today").to_string()
    } else if days < 60 {
        tw("member for {n} days", &[("n", &days)])
    } else if days < 730 {
        tw("member for {n} months", &[("n", &(days / 30))])
    } else {
        tw("member for {n} years", &[("n", &(days / 365))])
    }
}

pub fn date(at: i64) -> String {
    // Days since the epoch → a civil date (proleptic Gregorian).
    let days = at.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    match crate::i18n::lang() {
        crate::i18n::Lang::Portuguese => format!("{d:02}/{m:02}/{y}"),
        crate::i18n::Lang::English => format!("{y}-{m:02}-{d:02}"),
    }
}

pub fn medal_color(level: u8) -> Color {
    match level {
        0 => Color::new(0.80, 0.55, 0.33, 1.0),
        1 => Color::new(0.80, 0.82, 0.86, 1.0),
        2 => GOLD,
        _ => Color::new(0.62, 0.90, 0.98, 1.0),
    }
}

impl ProfileScreen {
    pub fn open(&mut self, name: &str, online: &mut Online) {
        self.name = name.to_string();
        self.history_first = 0;
        self.report = None;
        self.confirm_block = false;
        self.account = None;
        self.pane = Pane::Games;
        online.request_profile(name);
    }

    pub fn draw(&mut self, ui: &mut Ui, online: &mut Online) -> ProfileAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        let w = (sw - 40.0).min(960.0);
        let h = (sh - 40.0).min(680.0);
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
        ui.panel(r);
        let mut action = ProfileAction::None;
        if ui.icon_button(
            Rect::new(r.x + 14.0, r.y + 14.0, 34.0, 34.0),
            Icon::Back,
            t("Back"),
            true,
        ) || (is_key_pressed(KeyCode::Escape)
            && ui.focus.is_none()
            && !crate::countries::is_open())
        {
            action = ProfileAction::Back;
        }
        let Some(mut p) =
            online.profile.clone().filter(|p| p.player.username.eq_ignore_ascii_case(&self.name))
        else {
            ui.text_centered(
                t("Loading the profile…"),
                sw / 2.0,
                r.y + r.h / 2.0 - 10.0,
                16.0,
                MUTED,
                false,
            );
            return action;
        };
        // Asked before signing in: the server didn't know who was looking.
        if online.me.as_ref().is_some_and(|me| me.username.eq_ignore_ascii_case(&p.player.username))
        {
            p.relation = Relation::Me;
        }
        self.header(ui, online, &p, r);
        let body = Rect::new(r.x + 20.0, r.y + 204.0, r.w - 40.0, r.h - 224.0);
        let half = (body.w - 20.0) / 2.0;
        self.achievements(ui, &p, Rect::new(body.x, body.y, half, body.h));
        let right = Rect::new(body.x + half + 20.0, body.y, half, body.h);
        let panes = [t("Games"), t("Rating"), t("Numbers")];
        let selected = self.pane as usize;
        let seg = Rect::new(right.x + right.w - 270.0, right.y - 4.0, 270.0, 26.0);
        if let Some(k) = ui.segmented(seg, &panes, selected) {
            self.pane = [Pane::Games, Pane::Rating, Pane::Numbers][k];
        }
        match self.pane {
            Pane::Games => self.history(ui, online, &p, right),
            Pane::Rating => rating_chart(ui, &p, right),
            Pane::Numbers => numbers(ui, &p, right),
        }
        if let Some(code) = crate::countries::draw_picker(ui) {
            online.send(ClientMsg::SetCountry { country: code });
            online.request_profile(&p.player.username);
        }
        self.report_dialog(ui, online, &p);
        self.account_dialog(ui, online);
        action
    }

    fn header(&mut self, ui: &mut Ui, online: &mut Online, p: &Profile, r: Rect) {
        let player = &p.player;
        let x = r.x + 64.0;
        let mut nx = x;
        if crate::flags::draw(&player.country, x, r.y + 22.0, 26.0) {
            nx += crate::flags::width(26.0) + 12.0;
        }
        let name_w = ui.text_size(&player.username, 30.0, true).x;
        // The frame from the shop, around the flag and the name.
        if let Some(snook_proto::shop::Look::Frame { a, b, shine }) =
            snook_proto::shop::item(&player.frame).map(|i| i.look)
            && player.frame != "frame_none"
        {
            let fr = Rect::new(x - 12.0, r.y + 10.0, nx - x + name_w + 24.0, 48.0);
            crate::shop::frame(fr, a, b, shine, get_time() as f32);
            rrect(
                Rect::new(fr.x + 3.0, fr.y + 3.0, fr.w - 6.0, fr.h - 6.0),
                10.0,
                Color::new(0.09, 0.055, 0.035, 1.0),
            );
            crate::flags::draw(&player.country, x, r.y + 22.0, 26.0);
        }
        crate::ui::draw_name(ui, player, nx, r.y + 18.0, 30.0);
        let mut after = nx + name_w + 14.0;
        if player.vip {
            crate::icons::draw(
                crate::icons::Icon::Crown,
                vec2(after + 10.0, r.y + 34.0),
                22.0,
                GOLD,
            );
            after += 28.0;
        }
        if p.online {
            draw_circle(after, r.y + 36.0, 5.0, Color::new(0.35, 0.85, 0.45, 1.0));
        }
        let tier = player.tier();
        let mut info_x = x;
        if !player.guest {
            info_x += ui.pill(t(tier.name()), info_x, r.y + 60.0, tier_color(tier)) + 10.0;
        }
        let mut parts = Vec::new();
        if let Some(rank) = p.rank {
            parts.push(tw("#{n} in the world", &[("n", &rank)]));
        }
        if !player.country.is_empty() {
            parts.push(crate::flags::name(&player.country).to_string());
        }
        if !player.guest {
            parts.push(age(p.created_at));
        }
        // Friends see when the other was last around.
        if !p.online && p.last_seen > 0 {
            parts.push(crate::online::last_seen(p.last_seen));
        }
        ui.text(&parts.join("  ·  "), info_x, r.y + 61.0, 13.0, MUTED, false);

        // The rating, large, on the right.
        if !player.guest {
            let rating = player.rating.to_string();
            let rw = ui.text_size(&rating, 40.0, true).x;
            ui.text(&rating, r.x + r.w - rw - 24.0, r.y + 14.0, 40.0, tier_color(tier), true);
            let peak = tw("peak {n}", &[("n", &p.stats.peak_rating)]);
            let pw = ui.text_size(&peak, 12.0, false).x;
            ui.text(&peak, r.x + r.w - pw - 24.0, r.y + 62.0, 12.0, MUTED, false);
        }

        // Numbers.
        let s = &p.stats;
        let pct = (s.wins * 100).checked_div(s.games).unwrap_or(0);
        let cards = [
            (s.games.to_string(), t("rated games")),
            (s.wins.to_string(), t("wins")),
            (format!("{pct}%"), t("win rate")),
            (s.eight_wins.to_string(), t("won on the 8")),
            (s.best_streak.to_string(), t("best streak")),
            (s.tournaments_won.to_string(), t("tournaments won")),
        ];
        let cw = (r.w - 40.0 - 5.0 * 10.0) / 6.0;
        for (k, (n, label)) in cards.iter().enumerate() {
            let c = Rect::new(r.x + 20.0 + k as f32 * (cw + 10.0), r.y + 92.0, cw, 54.0);
            rrect(c, 8.0, Color::new(1.0, 1.0, 1.0, 0.04));
            ui.text_centered(n, c.x + c.w / 2.0, c.y + 6.0, 22.0, CREAM, true);
            let size = ui.fit(11.0, label, c.w - 6.0);
            ui.text_centered(label, c.x + c.w / 2.0, c.y + 34.0, size, MUTED, false);
        }

        // Progress towards the next tier.
        if !player.guest {
            let next = snook_proto::Tier::ALL.into_iter().find(|t| t.min_rating() > player.rating);
            let bx = r.x + 20.0;
            let bw = (r.w * 0.36).min(320.0);
            match next {
                Some(next) => {
                    let from = if tier == snook_proto::Tier::Rookie {
                        snook_core::rating::EloConfig::default().initial_rating.min(player.rating)
                    } else {
                        tier.min_rating()
                    };
                    let span = (next.min_rating() - from).max(1) as f32;
                    let k = ((player.rating - from) as f32 / span).clamp(0.0, 1.0);
                    let label = tw(
                        "{n} points to {tier}",
                        &[("n", &(next.min_rating() - player.rating)), ("tier", &t(next.name()))],
                    );
                    ui.text(&label, bx, r.y + 156.0, 12.0, MUTED, false);
                    let bar = Rect::new(bx, r.y + 175.0, bw, 6.0);
                    rrect(bar, 3.0, Color::new(1.0, 1.0, 1.0, 0.08));
                    if k > 0.0 {
                        rrect(
                            Rect::new(bar.x, bar.y, (bar.w * k).max(6.0), bar.h),
                            3.0,
                            tier_color(next),
                        );
                    }
                }
                None => ui.text(t("Top tier"), bx, r.y + 160.0, 13.0, tier_color(tier), true),
            }
        }

        draw_line(r.x + 20.0, r.y + 192.0, r.x + r.w - 20.0, r.y + 192.0, 1.0, BORDER);
        // Actions.
        let mut bx = r.x + r.w - 24.0;
        let by = r.y + 156.0;
        let mut button = |ui: &mut Ui, label: &str, enabled: bool| -> bool {
            let w = ui.text_size(label, 13.0, true).x + 26.0;
            bx -= w;
            let hit = ui.button(Rect::new(bx, by, w, 26.0), label, enabled);
            bx -= 8.0;
            hit
        };
        let name = player.username.clone();
        if player.guest || online.lan {
            return;
        }
        match p.relation {
            Relation::Me => {
                if button(ui, t("Account"), true) {
                    self.account = Some(AccountForm::default());
                    online.send(ClientMsg::Account);
                }
                if button(ui, t("Change country"), true) {
                    crate::countries::open();
                }
            }
            _ if online.me.is_none() => {}
            relation => {
                if relation != Relation::Blocked && button(ui, t("Report"), true) {
                    self.report = Some((0, String::new()));
                }
                let block_label = if relation == Relation::Blocked {
                    t("Unblock")
                } else if self.confirm_block {
                    t("Click again to block")
                } else {
                    t("Block")
                };
                if button(ui, block_label, true) {
                    if relation == Relation::Blocked {
                        online.send(ClientMsg::Unblock { username: name.clone() });
                        online.request_profile(&name);
                    } else if self.confirm_block {
                        online.send(ClientMsg::Block { username: name.clone() });
                        online.request_profile(&name);
                        self.confirm_block = false;
                    } else {
                        self.confirm_block = true;
                    }
                }
                let friend = match relation {
                    Relation::Friend => Some(t("Remove friend")),
                    Relation::RequestSent => Some(t("Request sent")),
                    Relation::RequestReceived => Some(t("Accept friend")),
                    Relation::None => Some(t("Add friend")),
                    _ => None,
                };
                if let Some(label) = friend {
                    let enabled = relation != Relation::RequestSent;
                    if button(ui, label, enabled) {
                        let msg = match relation {
                            Relation::Friend => ClientMsg::FriendRemove { username: name.clone() },
                            Relation::RequestReceived => {
                                ClientMsg::FriendAnswer { username: name.clone(), accept: true }
                            }
                            _ => ClientMsg::FriendRequest { username: name.clone() },
                        };
                        online.send(msg);
                        online.request_profile(&name);
                    }
                }
            }
        }
    }

    fn achievements(&self, ui: &mut Ui, p: &Profile, area: Rect) {
        let got =
            ACHIEVEMENTS.iter().filter(|a| p.achievements.iter().any(|(c, _)| c == a.code)).count();
        ui.text(
            &tw("Achievements {n}/{total}", &[("n", &got), ("total", &ACHIEVEMENTS.len())]),
            area.x,
            area.y,
            15.0,
            GOLD,
            true,
        );
        let cell = 40.0;
        let cols = ((area.w + 6.0) / (cell + 6.0)).floor().max(1.0) as usize;
        let rows = ACHIEVEMENTS.len().div_ceil(cols);
        // The reactions this player has unlocked, under the medals.
        let ry = area.y + 28.0 + rows as f32 * (cell + 6.0) + 14.0;
        if ry + 60.0 < area.y + area.h {
            let codes: Vec<&str> = p.achievements.iter().map(|(c, _)| c.as_str()).collect();
            let open = snook_proto::REACTIONS.iter().filter(|r| r.unlocked(&codes)).count();
            ui.text(
                &tw(
                    "Reactions {n}/{total}",
                    &[("n", &open), ("total", &snook_proto::REACTIONS.len())],
                ),
                area.x,
                ry,
                15.0,
                GOLD,
                true,
            );
            let size = 26.0;
            let per_row = ((area.w + 4.0) / (size + 4.0)).floor().max(1.0) as usize;
            for (k, r) in snook_proto::REACTIONS.iter().enumerate() {
                let c = vec2(
                    area.x + size / 2.0 + (k % per_row) as f32 * (size + 4.0),
                    ry + 26.0 + size / 2.0 + (k / per_row) as f32 * (size + 4.0),
                );
                if c.y + size / 2.0 > area.y + area.h {
                    break;
                }
                if r.unlocked(&codes) {
                    crate::reactions::draw(r.code, c, size, 1.0);
                } else {
                    crate::reactions::draw_locked(r.code, c, size * 0.9);
                }
            }
        }
        let mut tip = None;
        for (k, a) in ACHIEVEMENTS.iter().enumerate() {
            let (col, row) = (k % cols, k / cols);
            let c = Rect::new(
                area.x + col as f32 * (cell + 6.0),
                area.y + 28.0 + row as f32 * (cell + 6.0),
                cell,
                cell,
            );
            if c.y + c.h > area.y + area.h {
                break;
            }
            let unlocked = p.achievements.iter().find(|(code, _)| code == a.code);
            let color = medal_color(a.level);
            rrect(c, 8.0, Color::new(1.0, 1.0, 1.0, if unlocked.is_some() { 0.07 } else { 0.025 }));
            if unlocked.is_some() {
                rrect_lines(c, 8.0, 1.2, Color::new(color.r, color.g, color.b, 0.6));
                crate::icons::draw(
                    Icon::Medal,
                    vec2(c.x + cell / 2.0, c.y + cell / 2.0),
                    cell * 0.62,
                    color,
                );
            } else {
                // A faint medal: the colour hints at how hard it is.
                let ghost = Color::new(color.r * 0.5, color.g * 0.5, color.b * 0.5, 0.35);
                crate::icons::draw(
                    Icon::Medal,
                    vec2(c.x + cell / 2.0, c.y + cell / 2.0),
                    cell * 0.55,
                    ghost,
                );
            }
            ui.block(c);
            if ui.hovering(c) {
                tip = Some((a, unlocked.map(|(_, at)| *at), c));
            }
        }
        if let Some((a, at, c)) = tip {
            let title = t(a.title);
            let desc = t(a.description);
            let when = at
                .map(|at| tw("Unlocked {date}", &[("date", &date(at))]))
                .unwrap_or_else(|| t("Locked").to_string());
            let w = ui
                .text_size(title, 14.0, true)
                .x
                .max(ui.text_size(desc, 13.0, false).x)
                .max(ui.text_size(&when, 12.0, false).x)
                + 24.0;
            let tr = Rect::new(
                (c.x + c.w / 2.0 - w / 2.0).clamp(4.0, crate::ui::vw() - w - 4.0),
                c.y + c.h + 6.0,
                w,
                64.0,
            );
            rrect(tr, 8.0, Color::new(0.0, 0.0, 0.0, 0.9));
            rrect_lines(tr, 8.0, 1.0, medal_color(a.level));
            ui.text(title, tr.x + 12.0, tr.y + 8.0, 14.0, medal_color(a.level), true);
            ui.text(desc, tr.x + 12.0, tr.y + 28.0, 13.0, CREAM, false);
            ui.text(&when, tr.x + 12.0, tr.y + 46.0, 12.0, MUTED, false);
        }
    }

    fn history(&mut self, ui: &mut Ui, online: &mut Online, p: &Profile, area: Rect) {
        ui.text(t("Recent games"), area.x, area.y, 15.0, GOLD, true);
        let list = Rect::new(area.x, area.y + 28.0, area.w, area.h - 28.0);
        if p.history.is_empty() {
            ui.text(t("No games yet."), list.x, list.y + 6.0, 14.0, MUTED, false);
            return;
        }
        let row_h = 44.0;
        let visible = (list.h / row_h).floor().max(1.0) as usize;
        self.history_first = ui.scroll_list(list, self.history_first, p.history.len(), visible);
        for (k, h) in p.history.iter().skip(self.history_first).take(visible).enumerate() {
            let r = Rect::new(list.x, list.y + k as f32 * row_h, list.w - 8.0, row_h - 5.0);
            rrect(r, 6.0, Color::new(1.0, 1.0, 1.0, 0.035));
            let (mark, color) =
                if h.won { (t("W"), Color::new(0.45, 0.95, 0.55, 1.0)) } else { (t("L"), ALERT) };
            rrect(
                Rect::new(r.x + 6.0, r.y + 7.0, 24.0, 24.0),
                5.0,
                Color::new(color.r, color.g, color.b, 0.18),
            );
            ui.text_centered(mark, r.x + 18.0, r.y + 11.0, 14.0, color, true);
            let opp_color = tier_color(snook_proto::Tier::of(h.opponent_rating));
            ui.name_with_flag(
                &h.opponent,
                &h.opponent_country,
                opp_color,
                r.x + 40.0,
                r.y + 4.0,
                14.0,
                r.w - 150.0,
            );
            let how = crate::i18n::history_reason(h.reason, h.won);
            let sub = if h.tournament {
                format!("{how} · {}", t("tournament"))
            } else {
                how.to_string()
            };
            let sub = ui.ellipsize(&sub, r.w - 150.0, 12.0);
            ui.text(&sub, r.x + 40.0, r.y + 22.0, 12.0, MUTED, false);
            let delta = if h.rated {
                format!("{}{}", if h.delta >= 0 { "+" } else { "" }, h.delta)
            } else {
                t("casual").to_string()
            };
            let dc = if !h.rated {
                MUTED
            } else if h.delta >= 0 {
                Color::new(0.45, 0.95, 0.55, 1.0)
            } else {
                ALERT
            };
            let dw = ui.text_size(&delta, 15.0, true).x;
            ui.text(&delta, r.x + r.w - dw - 10.0, r.y + 4.0, 15.0, dc, true);
            // Watch it again (own games that kept their strokes).
            if p.relation == Relation::Me && h.replay && h.match_id > 0 {
                let br = Rect::new(r.x + r.w - dw - 46.0, r.y + 6.0, 26.0, 26.0);
                if ui.icon_button(br, Icon::Replay, t("Watch again"), true) {
                    online.send(ClientMsg::Replay { match_id: h.match_id });
                }
            }
            let d = date(h.at);
            let ddw = ui.text_size(&d, 11.0, false).x;
            ui.text(&d, r.x + r.w - ddw - 10.0, r.y + 23.0, 11.0, MUTED, false);
        }
    }

    /// Recovery e-mail, password and deleting the account.
    fn account_dialog(&mut self, ui: &mut Ui, online: &mut Online) {
        let Some(form) = self.account.as_mut() else { return };
        if !form.email_loaded
            && let Some(a) = &online.account
        {
            form.email = a.recovery_email.clone().or(a.payment_email.clone()).unwrap_or_default();
            form.email_loaded = true;
        }
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.55));
        let w = 480.0f32.min(sw - 20.0);
        let h = 530.0f32.min(sh - 20.0);
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
        ui.panel(r);
        ui.block(Rect::new(0.0, 0.0, sw, sh));
        ui.text(t("Your account"), r.x + 20.0, r.y + 16.0, 20.0, GOLD, true);
        let x = r.x + 20.0;
        let fw = w - 40.0;
        let mut y = r.y + 54.0;
        // Recovery e-mail.
        ui.text(t("Recovery e-mail"), x, y, 15.0, CREAM, true);
        y += 22.0;
        let note = t("If you forget your password, we confirm it's you by this e-mail.");
        for line in ui.wrap(note, fw, 12.0) {
            ui.text(&line, x, y, 12.0, MUTED, false);
            y += 16.0;
        }
        let saved = online.account.as_ref().and_then(|a| a.recovery_email.clone());
        ui.text_input(71, Rect::new(x, y + 2.0, fw - 110.0, 32.0), &mut form.email, 120);
        if ui.button(
            Rect::new(x + fw - 100.0, y + 2.0, 100.0, 32.0),
            t("Save"),
            saved.as_deref() != Some(form.email.trim()),
        ) {
            online.send(ClientMsg::SetRecoveryEmail { email: form.email.trim().to_string() });
        }
        y += 50.0;
        draw_line(x, y, x + fw, y, 1.0, BORDER);
        y += 12.0;
        // Password.
        ui.text(t("Change password"), x, y, 15.0, CREAM, true);
        y += 24.0;
        ui.text(t("Current"), x, y + 9.0, 13.0, MUTED, false);
        ui.password_input(
            72,
            Rect::new(x + 110.0, y, fw - 110.0, 32.0),
            &mut form.old_password,
            72,
        );
        y += 38.0;
        ui.text(t("New"), x, y + 9.0, 13.0, MUTED, false);
        ui.password_input(
            73,
            Rect::new(x + 110.0, y, fw - 110.0, 32.0),
            &mut form.new_password,
            72,
        );
        y += 38.0;
        let ok = snook_proto::valid_password(&form.new_password) && !form.old_password.is_empty();
        if ui.button(Rect::new(x + fw - 160.0, y, 160.0, 32.0), t("Change password"), ok) {
            online.send(ClientMsg::ChangePassword {
                old: std::mem::take(&mut form.old_password),
                new: std::mem::take(&mut form.new_password),
            });
        }
        y += 48.0;
        draw_line(x, y, x + fw, y, 1.0, BORDER);
        y += 12.0;
        // Deleting the account.
        ui.text(t("Delete account"), x, y, 15.0, ALERT, true);
        y += 22.0;
        let note = t(
            "Your name, e-mail, friends and items are erased for good. Your opponents keep the games you played (without your name).",
        );
        for line in ui.wrap(note, fw, 12.0) {
            ui.text(&line, x, y, 12.0, MUTED, false);
            y += 16.0;
        }
        ui.password_input(
            74,
            Rect::new(x, y + 2.0, fw - 170.0, 32.0),
            &mut form.delete_password,
            72,
        );
        if form.delete_password.is_empty() && ui.focus != Some(74) {
            ui.text(t("Password"), x + 9.0, y + 11.0, 14.0, MUTED, false);
        }
        let label =
            if form.confirm_delete { t("Click again to delete") } else { t("Delete account") };
        if ui.button(
            Rect::new(x + fw - 160.0, y + 2.0, 160.0, 32.0),
            label,
            !form.delete_password.is_empty(),
        ) {
            if form.confirm_delete {
                online.send(ClientMsg::DeleteAccount {
                    password: std::mem::take(&mut form.delete_password),
                });
                form.confirm_delete = false;
            } else {
                form.confirm_delete = true;
            }
        }
        if ui.button(
            Rect::new(r.x + (w - 140.0) / 2.0, r.y + h - 50.0, 140.0, 34.0),
            t("Close"),
            true,
        ) || (is_key_pressed(KeyCode::Escape) && ui.focus.is_none())
        {
            self.account = None;
            ui.focus = None;
        }
    }

    fn report_dialog(&mut self, ui: &mut Ui, online: &mut Online, p: &Profile) {
        let Some((reason, details)) = &mut self.report else { return };
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        let w = 420.0f32.min(sw - 20.0);
        let r = Rect::new((sw - w) / 2.0, sh / 2.0 - 160.0, w, 320.0);
        ui.panel(r);
        ui.block(Rect::new(0.0, 0.0, sw, sh));
        ui.text(
            &tw("Report {name}", &[("name", &p.player.username)]),
            r.x + 18.0,
            r.y + 16.0,
            18.0,
            GOLD,
            true,
        );
        let mut y = r.y + 50.0;
        for (k, rr) in ReportReason::ALL.iter().enumerate() {
            if ui.radio(r.x + 20.0, y, t(rr.name()), *reason == k) {
                *reason = k;
            }
            y += 26.0;
        }
        ui.text_input(61, Rect::new(r.x + 18.0, y + 4.0, w - 36.0, 32.0), details, 300);
        if details.is_empty() && ui.focus != Some(61) {
            ui.text(t("What happened? (optional)"), r.x + 27.0, y + 13.0, 14.0, MUTED, false);
        }
        let by = r.y + r.h - 50.0;
        let mut close = false;
        if ui.primary_button(Rect::new(r.x + w - 158.0, by, 140.0, 34.0), t("Send report")) {
            online.send(ClientMsg::Report {
                username: p.player.username.clone(),
                reason: ReportReason::ALL[*reason],
                details: details.clone(),
            });
            close = true;
        }
        if ui.button(Rect::new(r.x + 18.0, by, 120.0, 34.0), t("Cancel"), true) {
            close = true;
        }
        if close {
            self.report = None;
            ui.focus = None;
        }
    }
}

/// The rating after each rated game, as a line.
fn rating_chart(ui: &mut Ui, p: &Profile, area: Rect) {
    ui.text(t("Rating"), area.x, area.y, 15.0, GOLD, true);
    let pts = &p.rating_history;
    let chart = Rect::new(area.x + 40.0, area.y + 36.0, area.w - 50.0, area.h - 70.0);
    if pts.len() < 2 {
        ui.text(
            t("Play rated games to see your rating here."),
            area.x,
            area.y + 36.0,
            14.0,
            MUTED,
            false,
        );
        return;
    }
    let lo = pts.iter().map(|p| p.1).min().unwrap_or(0) - 15;
    let hi = pts.iter().map(|p| p.1).max().unwrap_or(0) + 15;
    let span = (hi - lo).max(1) as f32;
    let at = |k: usize, r: i32| {
        vec2(
            chart.x + chart.w * k as f32 / (pts.len() - 1) as f32,
            chart.y + chart.h * (1.0 - (r - lo) as f32 / span),
        )
    };
    rrect(chart, 6.0, Color::new(1.0, 1.0, 1.0, 0.03));
    // Tier lines that fall inside the chart.
    for tier in snook_proto::Tier::ALL {
        let m = tier.min_rating();
        if m > lo && m < hi {
            let y = chart.y + chart.h * (1.0 - (m - lo) as f32 / span);
            draw_line(chart.x, y, chart.x + chart.w, y, 1.0, Color::new(1.0, 1.0, 1.0, 0.08));
            ui.text(t(tier.name()), chart.x + 4.0, y - 14.0, 10.0, tier_color(tier), false);
        }
    }
    for (y, label) in [(chart.y, hi), (chart.y + chart.h - 12.0, lo)] {
        ui.text(&label.to_string(), area.x, y, 11.0, MUTED, false);
    }
    for k in 1..pts.len() {
        let (a, b) = (at(k - 1, pts[k - 1].1), at(k, pts[k].1));
        let color = if pts[k].1 >= pts[k - 1].1 { Color::new(0.45, 0.9, 0.55, 1.0) } else { ALERT };
        draw_line(a.x, a.y, b.x, b.y, 2.0, color);
    }
    let last = at(pts.len() - 1, pts[pts.len() - 1].1);
    draw_circle(last.x, last.y, 4.0, GOLD);
    let first = date(pts[0].0);
    let now = date(pts[pts.len() - 1].0);
    ui.text(&first, chart.x, chart.y + chart.h + 6.0, 11.0, MUTED, false);
    let nw = ui.text_size(&now, 11.0, false).x;
    ui.text(&now, chart.x + chart.w - nw, chart.y + chart.h + 6.0, 11.0, MUTED, false);
}

/// Lifetime numbers from the games with statistics, and games left unfinished.
fn numbers(ui: &mut Ui, p: &Profile, area: Rect) {
    ui.text(t("Numbers"), area.x, area.y, 15.0, GOLD, true);
    let c = &p.career;
    let per_game = c.balls_per_game_x10();
    let foul_rate = (c.fouls * 100).checked_div(c.shots).unwrap_or(0);
    let cards = [
        (format!("{}%", c.pot_rate()), t("potting success")),
        (format!("{}.{}", per_game / 10, per_game % 10), t("balls per game")),
        (c.best_run.to_string(), t("best run ever")),
        (format!("{foul_rate}%"), t("shots with a foul")),
        (c.games.to_string(), t("games counted")),
        (format!("{}%", p.abandon_rate), t("games left unfinished")),
    ];
    let cols = 2;
    let cw = (area.w - 10.0) / cols as f32;
    for (k, (n, label)) in cards.iter().enumerate() {
        let r = Rect::new(
            area.x + (k % cols) as f32 * (cw + 10.0),
            area.y + 34.0 + (k / cols) as f32 * 72.0,
            cw,
            62.0,
        );
        rrect(r, 8.0, Color::new(1.0, 1.0, 1.0, 0.04));
        let color = if k == 5 && p.abandon_rate >= 15 { ALERT } else { CREAM };
        ui.text_centered(n, r.x + r.w / 2.0, r.y + 8.0, 24.0, color, true);
        let size = ui.fit(12.0, label, r.w - 8.0);
        ui.text_centered(label, r.x + r.w / 2.0, r.y + 40.0, size, MUTED, false);
    }
}
