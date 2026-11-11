//! Local network games: host one (the game runs a small server) or join
//! one found on the network, or by address.

use macroquad::prelude::*;
use snook_proto::valid_username;
use snook_server::lan::{Discovery, LanHost};

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_rectangle};

use crate::config::Config;
use crate::i18n::{t, tw};
use crate::icons::Icon;
use crate::ui::{ALERT, BORDER, CREAM, GOLD, MUTED, Ui, rrect, rrect_lines};

pub enum LanAction {
    None,
    Back,
    /// Host a game as `name`.
    Host(String),
    /// Join the server at this address as `name`.
    Join(String, String),
}

pub struct LanScreen {
    discovery: Discovery,
    address: String,
    error: Option<String>,
    first: usize,
    /// The name field was filled in once (emptying it must not refill it).
    named: bool,
}

impl LanScreen {
    pub fn new() -> LanScreen {
        LanScreen {
            discovery: Discovery::new(),
            address: String::new(),
            error: None,
            first: 0,
            named: false,
        }
    }

    /// Starts hosting; returns the address to join.
    pub fn host(&mut self, name: &str) -> Option<(LanHost, String)> {
        match LanHost::start(name) {
            Ok(host) => {
                let url = format!("ws://127.0.0.1:{}/ws", host.port);
                Some((host, url))
            }
            Err(e) => {
                self.error = Some(tw("Could not host a game: {e}", &[("e", &e)]));
                None
            }
        }
    }

    pub fn draw(&mut self, ui: &mut Ui, config: &mut Config, hosting: Option<u16>) -> LanAction {
        self.discovery.poll();
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.45));
        let w = 560.0f32.min(sw - 20.0);
        let h = (sh - 40.0).min(560.0);
        let r = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, w, h);
        ui.panel(r);
        let mut action = LanAction::None;
        if ui.icon_button(
            Rect::new(r.x + 14.0, r.y + 14.0, 34.0, 34.0),
            Icon::Back,
            t("Back"),
            true,
        ) || (is_key_pressed(KeyCode::Escape) && ui.focus.is_none())
        {
            action = LanAction::Back;
        }
        crate::icons::draw(Icon::Network, vec2(r.x + 72.0, r.y + 31.0), 24.0, GOLD);
        ui.text(t("Local network"), r.x + 94.0, r.y + 18.0, 22.0, GOLD, true);
        let x = r.x + 28.0;
        let inner = w - 56.0;
        let mut y = r.y + 70.0;
        ui.text(t("Your name"), x, y, 13.0, MUTED, true);
        if !self.named && config.lan_name.is_empty() {
            config.lan_name = if config.username.is_empty() {
                config
                    .player_name
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(16)
                    .collect()
            } else {
                config.username.clone()
            };
            if config.lan_name.chars().count() < 3 {
                config.lan_name = format!("Player{}", crate::game::clock_seed() % 900 + 100);
            }
        }
        self.named = true;
        ui.text_input(50, Rect::new(x, y + 18.0, inner, 34.0), &mut config.lan_name, 16);
        let name_ok = valid_username(config.lan_name.trim());
        if !name_ok {
            ui.text(t("3–16 letters, digits or underscores."), x, y + 56.0, 12.0, ALERT, false);
        }
        y += 82.0;

        // Host.
        let host_r = Rect::new(x, y, inner, 44.0);
        if hosting.is_none() {
            let clicked = if name_ok {
                ui.primary_button(host_r, t("Host a game on this computer"))
            } else {
                ui.button(host_r, t("Host a game on this computer"), false)
            };
            if clicked {
                action = LanAction::Host(config.lan_name.trim().to_string());
            }
        } else {
            ui.button(host_r, t("You are hosting a game"), false);
        }
        y += 62.0;
        if let Some(port) = hosting {
            let ip = local_ip().unwrap_or_else(|| "?".into());
            ui.text(
                &tw("Others can join at {ip}:{port}", &[("ip", &ip), ("port", &port)]),
                x,
                y,
                13.0,
                CREAM,
                false,
            );
            y += 22.0;
        }

        // Games found.
        ui.text(t("Games on your network"), x, y, 15.0, GOLD, true);
        y += 26.0;
        let list = Rect::new(x, y, inner, (r.y + r.h - 130.0 - y).max(60.0));
        rrect(list, 6.0, Color::new(0.0, 0.0, 0.0, 0.25));
        rrect_lines(list, 6.0, 1.0, BORDER);
        let games = self.discovery.games.clone();
        if games.is_empty() {
            let text = if self.discovery.listening() {
                t("Looking for games…")
            } else {
                t("Can't listen for games here: join by address below.")
            };
            let dots = ".".repeat(((get_time() * 2.0) as usize) % 4);
            ui.text(
                &format!("{text}{}", if self.discovery.listening() { dots } else { String::new() }),
                list.x + 12.0,
                list.y + 12.0,
                14.0,
                MUTED,
                false,
            );
        }
        let row_h = 42.0;
        let visible = (list.h / row_h).floor().max(1.0) as usize;
        self.first = ui.scroll_list(list, self.first, games.len(), visible);
        for (k, g) in games.iter().skip(self.first).take(visible).enumerate() {
            let row = Rect::new(
                list.x + 6.0,
                list.y + 6.0 + k as f32 * row_h,
                list.w - 20.0,
                row_h - 6.0,
            );
            rrect(row, 6.0, Color::new(1.0, 1.0, 1.0, 0.05));
            crate::icons::draw(Icon::Table, vec2(row.x + 20.0, row.y + row.h / 2.0), 20.0, CREAM);
            ui.text(
                &tw("{host}'s game", &[("host", &g.host)]),
                row.x + 40.0,
                row.y + 4.0,
                15.0,
                CREAM,
                true,
            );
            ui.text(&g.address.to_string(), row.x + 40.0, row.y + 21.0, 11.0, MUTED, false);
            if ui.button(
                Rect::new(row.x + row.w - 90.0, row.y + 5.0, 84.0, row.h - 10.0),
                t("Join"),
                name_ok,
            ) {
                action = LanAction::Join(g.url(), config.lan_name.trim().to_string());
            }
        }
        y = list.y + list.h + 14.0;

        // By address.
        ui.text(t("Or join by address"), x, y, 13.0, MUTED, true);
        let input = Rect::new(x, y + 18.0, inner - 110.0, 34.0);
        let enter = ui.text_input(51, input, &mut self.address, 60);
        if self.address.is_empty() && ui.focus != Some(51) {
            ui.text("192.168.0.10", input.x + 9.0, input.y + 10.0, 14.0, MUTED, false);
        }
        let can = name_ok && !self.address.trim().is_empty();
        if (ui.button(Rect::new(x + inner - 100.0, y + 18.0, 100.0, 34.0), t("Join"), can) || enter)
            && can
        {
            action =
                LanAction::Join(address_url(&self.address), config.lan_name.trim().to_string());
        }
        y += 60.0;
        if let Some(e) = &self.error {
            ui.text(e, x, y, 12.0, ALERT, false);
        } else {
            ui.text(
                t("The first time, allow Snook through the firewall when asked."),
                x,
                y,
                12.0,
                MUTED,
                false,
            );
        }
        action
    }
}

/// "192.168.0.10" or "192.168.0.10:47879" → a WebSocket URL.
fn address_url(address: &str) -> String {
    let a = address.trim().trim_start_matches("ws://").trim_end_matches("/ws");
    if a.contains(':') {
        format!("ws://{a}/ws")
    } else {
        format!("ws://{a}:{}/ws", snook_proto::LAN_GAME_PORT)
    }
}

/// This computer's address on the local network.
fn local_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // No packet is sent: connecting a UDP socket only picks the route.
    sock.connect("10.255.255.255:1").ok()?;
    Some(sock.local_addr().ok()?.ip().to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn addresses_become_urls() {
        assert_eq!(super::address_url("10.0.0.2"), "ws://10.0.0.2:47879/ws");
        assert_eq!(super::address_url(" 10.0.0.2:9000 "), "ws://10.0.0.2:9000/ws");
    }
}
