//! The online part of the client: the connection (kept in the background
//! so the menu can show who is signed in), signing in, the lounge (players,
//! open tables, tournaments, friends, chat) and routing match messages to
//! the table. The same code runs LAN games, against a server hosted by one
//! of the players.

use std::collections::VecDeque;

use macroquad::prelude::*;

use crate::i18n::{t, tw};
use snookered_proto::{
    ClientMsg, FriendInfo, LeaderboardEntry, MAX_CHAT_LEN, PlayerInfo, PlayerStatus, Profile,
    ServerMsg, TableInfo, TournamentInfo, TournamentState, valid_password, valid_username,
};

#[allow(unused_imports)]
use crate::paint::{draw_circle, draw_line, draw_rectangle};

use crate::config::Config;
use crate::game::Match;
use crate::icons::Icon;
use crate::net::{Connection, NetEvent};
use crate::ui::{ALERT, BORDER, CREAM, FeedLine, GOLD, MUTED, Ui, name_color, rrect, rrect_lines};

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Not connected (no saved session, or the player left).
    Idle,
    /// Waiting for the socket to open.
    Connecting,
    /// The sign-in form.
    SignIn,
    /// Credentials sent, waiting for the answer.
    Authenticating,
    Lounge,
    /// The connection dropped while signed in; trying again.
    Reconnecting {
        attempt: u32,
        wait: f32,
    },
}

/// What the online screens ask of the rest of the game.
pub enum OnlineAction {
    None,
    /// Back to the main menu (the connection stays open).
    Leave,
    Profile(String),
    Ranking,
    Shop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Players,
    Tables,
    Tournaments,
    Friends,
}

#[derive(Default)]
struct TournamentForm {
    name: String,
    size: u32,
    rated: bool,
    password: String,
    /// Coins to enter (0: free).
    entry_fee: u32,
    format: snookered_proto::TournamentFormat,
}

#[derive(Default)]
pub struct FriendsData {
    pub friends: Vec<FriendInfo>,
    pub incoming: Vec<String>,
    pub outgoing: Vec<String>,
    pub blocked: Vec<String>,
}

/// The player's shop state, as the server last sent it.
#[derive(Debug, Clone, Default)]
pub struct WalletView {
    pub coins: i64,
    pub vip_until: i64,
    pub owned: Vec<String>,
    pub equipped: Vec<(String, String)>,
}

impl WalletView {
    pub fn is_vip(&self) -> bool {
        self.vip_until > crate::game::unix_now()
    }
}

/// What the server sells for money (empty when it takes no payments).
#[derive(Debug, Clone, Default)]
pub struct ShopOffers {
    pub payments: bool,
    pub packs: Vec<snookered_proto::PackOffer>,
    pub vip_price: String,
    pub vip_coins: u32,
    /// The price of a VIP month given to a friend (empty: not on sale).
    pub vip_gift_price: String,
}

pub struct Online {
    conn: Option<Connection>,
    pub wallet: Option<WalletView>,
    pub offers: Option<ShopOffers>,
    /// Set when the items in use changed (the table must be redrawn).
    pub looks_changed: bool,
    /// The cloth and ball set of the current online table (shop codes;
    /// empty: this player's own).
    pub table_look: [String; 2],
    /// This account's achievements (for the reactions they unlock).
    achievements: Vec<String>,
    pub stage: Stage,
    pub server: String,
    /// A LAN game (guest name, nothing saved).
    pub lan: bool,
    username: String,
    password: String,
    country: String,
    register: bool,
    /// Error shown on the sign-in form.
    pub error: Option<String>,
    /// Short messages at the bottom of the screen.
    toasts: VecDeque<(String, f32, bool)>,
    pub me: Option<PlayerInfo>,
    players: Vec<PlayerInfo>,
    chat: Vec<FeedLine>,
    chat_input: String,
    chat_scroll: usize,
    pub leaderboard: Vec<LeaderboardEntry>,
    pub leaderboard_loaded: bool,
    leaderboard_age: f32,
    incoming: Vec<(String, bool)>,
    outgoing: Option<(String, bool)>,
    motd: String,
    token: String,
    auto_resume: bool,
    rated_games: bool,
    pub tab: Tab,
    list_first: usize,
    tables: Vec<TableInfo>,
    table_private: bool,
    table_password: String,
    joining: Option<(u64, String)>,
    /// An invitation code being typed.
    code_input: String,
    tournaments: Vec<TournamentInfo>,
    selected_tournament: Option<u64>,
    tournament_form: Option<TournamentForm>,
    tournament_password: String,
    pub friends: FriendsData,
    friend_input: String,
    pub profile: Option<Profile>,
    unread_friend_requests: bool,
    /// Searching in "Play now": rated, seconds waited and the rating range
    /// when the server last said so, and when that was.
    queue: Option<(bool, u32, u32, std::time::Instant)>,
    /// Round trip to the server, measured with a ping every few seconds.
    pub rtt: Option<std::time::Duration>,
    ping_sent: Option<std::time::Instant>,
    ping_timer: f32,
    /// A purchase sent to the server and not answered yet (item code).
    pub buying: Option<String>,
    /// The last purchase the server confirmed: (item, given to).
    pub bought: Option<(String, Option<String>)>,
    /// The last thing the server refused while buying.
    pub shop_error: Option<String>,
    /// The account page, as the server last sent it.
    pub account: Option<AccountView>,
    /// The e-mail of a payment, offered as the recovery e-mail.
    pub email_offer: Option<String>,
    /// Games that can be watched again, and how many are kept (0: all).
    pub replays: Option<(Vec<snookered_proto::HistoryEntry>, u32)>,
    /// A replay to watch (opened by the replays screen).
    pub replay: Option<Box<snookered_proto::ReplayInfo>>,
    /// The game on the lounge's TV.
    pub featured: Option<snookered_proto::FeaturedMatch>,
    /// A code friends can use to sit at this player's table.
    pub invite_code: Option<String>,
    /// The server restarts at this time (no new games until then).
    pub draining: Option<std::time::Instant>,
    /// Something happened that a player in another window should notice.
    pub attention: Option<String>,
    /// What the right side of the lounge shows.
    side: Side,
    /// Today's and this week's missions.
    missions: Option<MissionsView>,
    /// A mission was finished since the missions were last looked at.
    mission_done: bool,
    /// The notes on the lounge's board, newest first.
    board: Vec<snookered_proto::BoardNote>,
    /// Scheduled tournaments the player was already told are about to start.
    told_starting: Vec<u64>,
}

/// "5 h", "3 days", "12 min": how long until something.
fn time_left(secs: i64) -> String {
    let secs = secs.max(0);
    if secs >= 2 * 86_400 {
        tw("{n} days", &[("n", &(secs / 86_400))])
    } else if secs >= 3600 {
        tw("{n} h", &[("n", &(secs / 3600))])
    } else {
        tw("{n} min", &[("n", &(secs / 60).max(1))])
    }
}

/// The right side of the lounge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Chat,
    Missions,
    Board,
}

struct MissionsView {
    daily: Vec<snookered_proto::missions::MissionState>,
    weekly: Vec<snookered_proto::missions::MissionState>,
    day_ends: i64,
    week_ends: i64,
}

/// The account page.
#[derive(Debug, Clone, Default)]
pub struct AccountView {
    pub recovery_email: Option<String>,
    pub payment_email: Option<String>,
}

const MAX_RECONNECTS: u32 = 15;

impl Online {
    /// The global server. Connects (and signs in) at once if a session was saved.
    pub fn new(config: &Config) -> Online {
        let mut o = Online::empty(&config.server, false);
        o.username = config.username.clone();
        o.token = config.token.clone();
        o.auto_resume = !o.token.is_empty();
        o.country = crate::flags::system_country();
        if o.auto_resume {
            o.connect();
        }
        o
    }

    /// A LAN server, joined as the guest `name`.
    pub fn lan(url: &str, name: &str) -> Online {
        let mut o = Online::empty(url, true);
        o.username = name.to_string();
        o.rated_games = false;
        o.connect();
        o
    }

    fn empty(server: &str, lan: bool) -> Online {
        Online {
            conn: None,
            stage: Stage::Idle,
            server: server.to_string(),
            lan,
            username: String::new(),
            password: String::new(),
            country: String::new(),
            register: false,
            error: None,
            toasts: VecDeque::new(),
            me: None,
            players: Vec::new(),
            chat: Vec::new(),
            chat_input: String::new(),
            chat_scroll: 0,
            leaderboard: Vec::new(),
            leaderboard_loaded: false,
            leaderboard_age: f32::MAX,
            incoming: Vec::new(),
            outgoing: None,
            motd: String::new(),
            token: String::new(),
            auto_resume: false,
            rated_games: true,
            tab: Tab::Players,
            list_first: 0,
            tables: Vec::new(),
            table_private: false,
            table_password: String::new(),
            joining: None,
            code_input: String::new(),
            tournaments: Vec::new(),
            selected_tournament: None,
            tournament_form: None,
            tournament_password: String::new(),
            friends: FriendsData::default(),
            friend_input: String::new(),
            profile: None,
            unread_friend_requests: false,
            queue: None,
            wallet: None,
            offers: None,
            looks_changed: false,
            table_look: Default::default(),
            achievements: Vec::new(),
            rtt: None,
            ping_sent: None,
            ping_timer: 0.0,
            buying: None,
            bought: None,
            shop_error: None,
            account: None,
            email_offer: None,
            replays: None,
            replay: None,
            featured: None,
            invite_code: None,
            draining: None,
            attention: None,
            // Development aid: SNOOKERED_LOUNGE_SIDE=missions|board.
            side: match crate::dev::var("SNOOKERED_LOUNGE_SIDE").as_deref() {
                Some("missions") => Side::Missions,
                Some("board") => Side::Board,
                _ => Side::Chat,
            },
            missions: None,
            mission_done: false,
            board: Vec::new(),
            told_starting: Vec::new(),
        }
    }

    pub fn send(&self, msg: ClientMsg) {
        if let Some(c) = &self.conn {
            c.send(msg);
        }
    }

    pub fn signed_in(&self) -> bool {
        self.me.is_some()
    }

    pub fn has_session(&self) -> bool {
        !self.token.is_empty()
    }

    /// Opens the connection if it isn't open (for the ranking, a profile or the lounge).
    pub fn connect(&mut self) {
        if self.conn.is_none() {
            self.conn = Some(Connection::open(&self.server));
            if !matches!(self.stage, Stage::Reconnecting { .. }) {
                self.stage = Stage::Connecting;
            }
        }
    }

    /// Number of invitations waiting for an answer (shown during matches).
    pub fn pending_invites(&self) -> usize {
        self.incoming.len()
    }

    pub fn toast(&mut self, text: impl Into<String>, good: bool) {
        self.toasts.push_back((text.into(), 5.0, good));
        while self.toasts.len() > 3 {
            self.toasts.pop_front();
        }
    }

    pub fn request_leaderboard(&mut self) {
        self.connect();
        self.send(ClientMsg::Leaderboard);
        self.leaderboard_age = 0.0;
    }

    pub fn request_profile(&mut self, name: &str) {
        self.connect();
        if self.profile.as_ref().is_none_or(|p| !p.player.username.eq_ignore_ascii_case(name)) {
            self.profile = None;
        }
        self.send(ClientMsg::Profile { username: name.to_string() });
    }

    /// Handles network traffic. Creates, updates or ends `current` when a match
    /// message arrives. Returns true if the config changed and should be saved.
    pub fn update(&mut self, dt: f32, current: &mut Option<Match>, config: &mut Config) -> bool {
        let mut save = false;
        for toast in &mut self.toasts {
            toast.1 -= dt;
        }
        self.toasts.retain(|t| t.1 > 0.0);
        self.tournaments_starting();
        // Outgoing match messages.
        if let Some(m) = current.as_mut()
            && let Some(o) = m.online.as_mut()
        {
            // Keep messages while reconnecting; they go out once signed in.
            if let Some(c) = &self.conn
                && self.signed_in()
            {
                for msg in o.outbox.drain(..) {
                    c.send(msg);
                }
            }
        }
        if let Stage::Reconnecting { attempt, wait } = &mut self.stage {
            *wait -= dt;
            if *wait <= 0.0 && self.conn.is_none() {
                *attempt += 1;
                self.conn = Some(Connection::open(&self.server));
            }
        }
        self.leaderboard_age += dt;
        // Measure the round trip every few seconds while signed in.
        if self.signed_in() {
            self.ping_timer -= dt;
            if self.ping_timer <= 0.0 {
                self.ping_timer = 3.0;
                if self.ping_sent.is_none_or(|t| t.elapsed().as_secs() > 10) {
                    self.ping_sent = Some(std::time::Instant::now());
                    self.send(ClientMsg::Ping);
                }
            }
        }

        while let Some(event) = self.conn.as_mut().and_then(|c| c.poll()) {
            match event {
                NetEvent::Connected => {
                    if self.lan {
                        self.send(ClientMsg::Guest { name: self.username.clone() });
                        self.stage = Stage::Authenticating;
                    } else if self.auto_resume && !self.token.is_empty() {
                        self.send(ClientMsg::Resume { token: self.token.clone() });
                        if !matches!(self.stage, Stage::Reconnecting { .. }) {
                            self.stage = Stage::Authenticating;
                        }
                    } else {
                        self.stage = Stage::SignIn;
                    }
                }
                NetEvent::Closed(reason) => {
                    self.conn = None;
                    self.queue = None;
                    let was_in = self.signed_in();
                    let attempt = match self.stage {
                        Stage::Reconnecting { attempt, .. } => attempt,
                        _ => 0,
                    };
                    let can_resume = self.lan || !self.token.is_empty();
                    if was_in && can_resume && self.auto_resume && attempt < MAX_RECONNECTS {
                        if attempt == 0
                            && let Some(m) = current.as_mut()
                        {
                            m.notify(t("Connection lost. Reconnecting…"), true);
                        }
                        let wait = (1.0 + attempt as f32 * 1.5).min(8.0);
                        self.stage = Stage::Reconnecting { attempt, wait };
                    } else {
                        if let Some(m) = current.take_if(|m| m.is_online()) {
                            drop(m);
                        }
                        self.me = None;
                        self.stage = if self.lan { Stage::Idle } else { Stage::SignIn };
                        if self.error.is_none() {
                            self.error = Some(crate::i18n::server_text(&reason));
                        }
                    }
                }
                NetEvent::Message(msg) => save |= self.handle(*msg, current, config),
            }
        }
        save
    }

    fn handle(&mut self, msg: ServerMsg, current: &mut Option<Match>, config: &mut Config) -> bool {
        let mut save = false;
        match msg {
            ServerMsg::AuthOk { player, token } => {
                if let Some(m) = current.as_mut()
                    && matches!(self.stage, Stage::Reconnecting { .. })
                {
                    m.notify(t("Reconnected."), false);
                }
                self.username = player.username.clone();
                self.me = Some(player);
                self.auto_resume = true;
                self.password.clear();
                self.error = None;
                self.stage = Stage::Lounge;
                if !self.lan {
                    self.token = token.clone();
                    config.username = self.username.clone();
                    config.token = token;
                    config.server = self.server.clone();
                    save = true;
                    self.send(ClientMsg::Friends);
                    self.send(ClientMsg::Leaderboard);
                    // Development aid: SNOOKERED_AUTOINVITE=<name> invites that player.
                    if let Some(to) = crate::dev::var("SNOOKERED_AUTOINVITE") {
                        self.send(ClientMsg::Invite { to, rated: true });
                    }
                    // SNOOKERED_AUTOQUEUE=1 joins "Play now".
                    if crate::dev::flag("SNOOKERED_AUTOQUEUE") {
                        self.send(ClientMsg::QueueJoin { rated: true });
                    }
                    // SNOOKERED_AUTOWATCH=<name> watches that friend's game.
                    if let Some(username) = crate::dev::var("SNOOKERED_AUTOWATCH") {
                        self.send(ClientMsg::Spectate { username });
                    }
                }
            }
            ServerMsg::AuthError { reason } => {
                if self.auto_resume && !self.token.is_empty() {
                    // The saved session expired (or the account was banned).
                    self.token.clear();
                    config.token.clear();
                    save = true;
                }
                self.auto_resume = false;
                self.me = None;
                if let Some(m) = current.take_if(|m| m.is_online()) {
                    drop(m);
                }
                self.stage = if self.lan { Stage::Idle } else { Stage::SignIn };
                self.error = Some(crate::i18n::server_text(&reason));
            }
            ServerMsg::Kicked { reason } => {
                // Signed in elsewhere, banned, or disconnected by the staff:
                // don't fight over the account by reconnecting.
                self.auto_resume = false;
                self.me = None;
                if let Some(m) = current.take_if(|m| m.is_online()) {
                    drop(m);
                }
                self.conn = None;
                self.queue = None;
                self.stage = if self.lan { Stage::Idle } else { Stage::SignIn };
                self.error = Some(crate::i18n::server_text(&reason));
            }
            ServerMsg::Welcome { motd, .. } => self.motd = crate::i18n::server_text(&motd),
            ServerMsg::Lounge { players } => self.players = players,
            ServerMsg::PlayerUpdated { player } => {
                if self.me.as_ref().is_some_and(|me| me.username == player.username) {
                    self.me = Some(player.clone());
                }
                for f in &mut self.friends.friends {
                    if f.player.username == player.username {
                        f.player = player.clone();
                        f.online = true;
                    }
                }
                match self.players.iter_mut().find(|p| p.username == player.username) {
                    Some(p) => *p = player,
                    None => {
                        // A friend just came online.
                        if self.friends.friends.iter().any(|f| f.player.username == player.username)
                        {
                            self.attention =
                                Some(tw("{name} is online.", &[("name", &player.username)]));
                        }
                        self.players.push(player)
                    }
                }
            }
            ServerMsg::PlayerLeft { username } => {
                self.players.retain(|p| p.username != username);
                self.incoming.retain(|(from, _)| *from != username);
                for f in &mut self.friends.friends {
                    if f.player.username == username {
                        f.online = false;
                    }
                }
                if self.outgoing.as_ref().is_some_and(|(to, _)| *to == username) {
                    self.outgoing = None;
                }
            }
            ServerMsg::ChatHistory { lines } => {
                self.chat = lines.iter().map(FeedLine::chat).collect();
                self.chat_scroll = 0;
            }
            ServerMsg::Chat { line } => {
                if !self.friends.blocked.iter().any(|b| b.eq_ignore_ascii_case(&line.from)) {
                    self.chat.push(FeedLine::chat(&line));
                    if self.chat.len() > 200 {
                        self.chat.remove(0);
                    }
                    if self.chat_scroll > 0 {
                        self.chat_scroll += 1;
                    }
                }
            }
            ServerMsg::Invited { from, rated } => {
                if crate::dev::flag("SNOOKERED_AUTOPLAY") && current.is_none() {
                    // Development aid: accept at once.
                    self.send(ClientMsg::AnswerInvite { from, accept: true });
                    return save;
                }
                self.attention = Some(tw("{name} invites you to play.", &[("name", &from)]));
                self.incoming.retain(|(f, _)| *f != from);
                self.incoming.push((from, rated));
            }
            ServerMsg::InviteSent { to, rated } => self.outgoing = Some((to, rated)),
            ServerMsg::InviteCancelled { from } => self.incoming.retain(|(f, _)| *f != from),
            ServerMsg::InviteDeclined { by } => {
                self.outgoing = None;
                self.toast(tw("{by} declined your invitation.", &[("by", &by)]), false);
            }
            ServerMsg::InviteFailed { reason } => {
                self.outgoing = None;
                self.toast(crate::i18n::server_text(&reason), false);
            }
            ServerMsg::Tables { tables } => self.tables = tables,
            ServerMsg::TableOpened { table } => {
                self.tables.retain(|t| t.id != table.id);
                self.tables.push(table);
            }
            ServerMsg::TableClosed { id } => self.tables.retain(|t| t.id != id),
            ServerMsg::Tournaments { tournaments } => self.tournaments = tournaments,
            ServerMsg::TournamentUpdated { tournament } => {
                // A tournament this player is in has just begun.
                let me = self.me.as_ref().map(|m| m.username.as_str()).unwrap_or("");
                let was = self.tournaments.iter().find(|t| t.id == tournament.id);
                if was.is_some_and(|t| t.state == TournamentState::Registering)
                    && tournament.state == TournamentState::Running
                    && tournament.players.iter().any(|p| p.username == me)
                {
                    self.attention = Some(tw("{name} has started!", &[("name", &tournament.name)]));
                }
                match self.tournaments.iter_mut().find(|t| t.id == tournament.id) {
                    Some(t) => *t = tournament,
                    None => self.tournaments.push(tournament),
                }
            }
            ServerMsg::TournamentRemoved { id } => {
                self.tournaments.retain(|t| t.id != id);
                if self.selected_tournament == Some(id) {
                    self.selected_tournament = None;
                }
            }
            ServerMsg::Leaderboard { entries } => {
                self.leaderboard = entries;
                self.leaderboard_loaded = true;
            }
            ServerMsg::Profile { profile } => self.profile = Some(*profile),
            ServerMsg::Friends { friends, incoming, outgoing, blocked } => {
                self.friends = FriendsData { friends, incoming, outgoing, blocked };
                if self.friends.incoming.is_empty() {
                    self.unread_friend_requests = false;
                }
            }
            ServerMsg::FriendRequestReceived { from } => {
                self.unread_friend_requests = true;
                self.toast(tw("{name} wants to be your friend.", &[("name", &from)]), true);
            }
            ServerMsg::AchievementUnlocked { code } => {
                let title = snookered_proto::ACHIEVEMENTS
                    .iter()
                    .find(|a| a.code == code)
                    .map(|a| t(a.title).to_string())
                    .unwrap_or(code);
                self.toast(tw("Achievement unlocked: {title}", &[("title", &title)]), true);
            }
            ServerMsg::Notice { text } => self.toast(crate::i18n::server_text(&text), true),
            ServerMsg::MatchStarted {
                match_id,
                seat,
                players,
                rated,
                game,
                shot_clock_secs,
                clock_left,
                tournament,
                spectating,
                table_look,
            } => {
                self.table_look = table_look;
                if !spectating && current.as_ref().is_none_or(|m| m.match_id() != match_id) {
                    let other = &players[1 - seat.min(1)].username;
                    self.attention =
                        Some(tw("Your game against {name} is starting.", &[("name", other)]));
                }
                self.outgoing = None;
                self.queue = None;
                if !spectating {
                    self.incoming.clear();
                }
                // Development aid: SNOOKERED_AUTOCHAT="text" says it at the start of a match.
                if let Some(text) = crate::dev::var("SNOOKERED_AUTOCHAT").filter(|_| !spectating) {
                    self.send(ClientMsg::MatchChat { text });
                }
                // SNOOKERED_AUTOREACT="code" sends that reaction.
                if let Some(code) = crate::dev::var("SNOOKERED_AUTOREACT") {
                    self.send(ClientMsg::React { code });
                }
                let next = Match::online(
                    match_id,
                    seat,
                    &players,
                    rated,
                    game,
                    shot_clock_secs,
                    clock_left,
                    tournament,
                    config.guide,
                    spectating,
                );
                match current.as_mut() {
                    // Same match after reconnecting, or a rematch with the same opponent.
                    Some(m) if m.is_online() && same_players(m, &next) => {
                        let same_game = m.match_id() == match_id;
                        m.continue_online(next, same_game)
                    }
                    _ => *current = Some(next),
                }
                if let Some(o) = current.as_mut().and_then(|m| m.online.as_mut()) {
                    o.unlocked = self.achievements.clone();
                }
                // A message sent just before the match (e.g. "this game is casual").
                if let (Some((text, ..)), Some(m)) = (self.toasts.pop_back(), current.as_mut()) {
                    m.notify(text, true);
                }
            }
            ServerMsg::Error { message } if self.buying.is_some() => {
                self.buying = None;
                self.shop_error = Some(crate::i18n::server_text(&message));
            }
            ServerMsg::Error { message } => match current.as_mut() {
                Some(m) if m.is_online() => m.handle_server(ServerMsg::Error { message }),
                _ => self.toast(crate::i18n::server_text(&message), false),
            },
            ServerMsg::ItemBought { code, gift_to } => {
                self.buying = None;
                self.bought = Some((code, gift_to));
            }
            ServerMsg::GiftReceived { from, code } => {
                let name = snookered_proto::shop::item(&code).map(|i| t(i.name)).unwrap_or(&code);
                self.toast(
                    tw("{from} gave you {item}!", &[("from", &from), ("item", &name)]),
                    true,
                );
                self.attention =
                    Some(tw("{from} gave you {item}!", &[("from", &from), ("item", &name)]));
            }
            ServerMsg::CoinsEarned { amount, reason } => {
                let why = match reason.as_str() {
                    "daily" => t("daily bonus"),
                    "win" => t("for the win"),
                    "game" => t("for the game"),
                    "prize" => t("tournament prize"),
                    "refund" => t("entry fee back"),
                    "mission" => {
                        if self.side != Side::Missions {
                            self.mission_done = true;
                        }
                        t("mission complete")
                    }
                    _ => "",
                };
                self.toast(tw("+{n} coins · {why}", &[("n", &amount), ("why", &why)]), true);
            }
            ServerMsg::Account { recovery_email, payment_email } => {
                // After a payment, without a recovery e-mail: offer the one paid with.
                if recovery_email.is_none() && !config.email_offer_declined {
                    self.email_offer = payment_email.clone();
                }
                self.account = Some(AccountView { recovery_email, payment_email });
            }
            ServerMsg::AccountDeleted => {
                self.auto_resume = false;
                self.me = None;
                self.conn = None;
                self.token.clear();
                config.token.clear();
                config.username.clear();
                save = true;
                self.stage = Stage::SignIn;
                self.error = Some(t("Your account was deleted.").to_string());
            }
            ServerMsg::Replays { games, limit } => self.replays = Some((games, limit)),
            ServerMsg::Replay { replay } => self.replay = Some(replay),
            ServerMsg::Featured { featured } => self.featured = featured,
            ServerMsg::Missions { daily, weekly, day_ends, week_ends } => {
                self.missions = Some(MissionsView { daily, weekly, day_ends, week_ends });
            }
            ServerMsg::Board { notes } => self.board = notes,
            ServerMsg::InviteCode { code } => self.invite_code = Some(code),
            ServerMsg::Draining { secs } => {
                self.draining =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(secs as u64));
            }
            ServerMsg::Wallet { coins, vip_until, owned, equipped } => {
                let wallet = WalletView { coins, vip_until, owned, equipped };
                // The items in use, as the server allows them now.
                for c in snookered_proto::shop::Category::ALL {
                    let code = self.effective(&wallet, c).to_string();
                    let slot = match c {
                        snookered_proto::shop::Category::Cloth => &mut config.cloth,
                        snookered_proto::shop::Category::Cue => &mut config.cue,
                        snookered_proto::shop::Category::Chalk => &mut config.chalk,
                        snookered_proto::shop::Category::Balls => &mut config.balls,
                        snookered_proto::shop::Category::Effect => &mut config.effect,
                        // Only seen online.
                        snookered_proto::shop::Category::Frame
                        | snookered_proto::shop::Category::Name => {
                            continue;
                        }
                    };
                    if *slot != code {
                        *slot = code;
                        self.looks_changed = true;
                        save = true;
                    }
                }
                self.wallet = Some(wallet);
            }
            ServerMsg::ShopInfo { payments, packs, vip_price, vip_coins, vip_gift_price } => {
                self.offers =
                    Some(ShopOffers { payments, packs, vip_price, vip_coins, vip_gift_price });
            }
            ServerMsg::VipGift { from, to } => {
                let text = if self.me.as_ref().is_some_and(|me| me.username == to) {
                    tw("{from} gave you a month of VIP!", &[("from", &from)])
                } else {
                    tw("{name} received a month of VIP.", &[("name", &to)])
                };
                self.toast(text.clone(), true);
                self.attention = Some(text);
            }
            // The Store build asks for none, and opens none.
            ServerMsg::PaymentPage { .. } if !crate::channel::real_money_purchases() => {}
            ServerMsg::PaymentPage { url } => {
                if crate::util::open_url(&url) {
                    self.toast(t("The payment page opened in your browser."), false);
                } else {
                    self.toast(t("Could not open the browser."), true);
                }
            }
            ServerMsg::Queue { searching, rated, waited_secs, range } => {
                self.queue =
                    searching.then(|| (rated, waited_secs, range, std::time::Instant::now()));
            }
            ServerMsg::SpectateEnded { reason } => {
                if current.as_ref().is_some_and(|m| m.is_spectating()) {
                    *current = None;
                }
                self.toast(crate::i18n::server_text(&reason), false);
            }
            ServerMsg::MyAchievements { codes } => {
                if let Some(o) = current.as_mut().and_then(|m| m.online.as_mut()) {
                    o.unlocked = codes.clone();
                }
                self.achievements = codes;
            }
            ServerMsg::Pong => {
                if let Some(sent) = self.ping_sent.take() {
                    self.rtt = Some(sent.elapsed());
                }
            }
            other => {
                if let Some(m) = current.as_mut().filter(|m| m.is_online()) {
                    m.handle_server(other);
                }
            }
        }
        save
    }

    /// Called when the player leaves the table to go back to the lounge.
    /// The item used in `category` (what is equipped, if still allowed).
    pub fn effective(
        &self,
        w: &WalletView,
        category: snookered_proto::shop::Category,
    ) -> &'static str {
        use snookered_proto::shop;
        w.equipped
            .iter()
            .find(|(c, _)| c == category.code())
            .and_then(|(_, code)| shop::item(code))
            .filter(|i| {
                i.category == category && shop::can_use(i, &w.owned, &self.achievements, w.is_vip())
            })
            .map(|i| i.code)
            .unwrap_or_else(|| category.default_item())
    }

    /// Whether this player may use `item` now.
    pub fn can_use(&self, item: &snookered_proto::shop::Item) -> bool {
        let w = self.wallet.clone().unwrap_or_default();
        snookered_proto::shop::can_use(item, &w.owned, &self.achievements, w.is_vip())
    }

    /// Starts watching a friend's game.
    pub fn spectate(&mut self, name: &str) {
        self.send(ClientMsg::Spectate { username: name.to_string() });
    }

    pub fn leave_match(&mut self, m: &Match) {
        if m.is_spectating() {
            self.send(ClientMsg::StopSpectating);
            return;
        }
        if m.game.game_over().is_none() {
            self.send(ClientMsg::Resign);
        }
        self.send(ClientMsg::LeaveMatch);
    }

    pub fn logout(&mut self, config: &mut Config) {
        self.send(ClientMsg::Logout);
        self.conn = None;
        self.queue = None;
        self.me = None;
        self.token.clear();
        self.auto_resume = false;
        self.friends = FriendsData::default();
        config.token.clear();
        config.save();
        self.stage = Stage::Idle;
        self.connect();
    }

    /// After a payment: "use this e-mail to recover your account?". Returns
    /// true if the settings changed.
    pub fn draw_email_offer(&mut self, ui: &mut Ui, config: &mut Config) -> bool {
        let Some(email) = self.email_offer.clone() else { return false };
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let w = 460.0f32.min(sw - 20.0);
        let r = Rect::new((sw - w) / 2.0, sh / 2.0 - 110.0, w, 220.0);
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5));
        ui.panel(r);
        ui.block(Rect::new(0.0, 0.0, sw, sh));
        ui.text(t("Protect your purchases"), r.x + 20.0, r.y + 18.0, 19.0, GOLD, true);
        let text = tw(
            "Use {email} to recover your account if you ever forget your password?",
            &[("email", &email)],
        );
        for (k, line) in ui.wrap(&text, w - 40.0, 14.0).iter().enumerate() {
            ui.text(line, r.x + 20.0, r.y + 56.0 + k as f32 * 19.0, 14.0, CREAM, false);
        }
        ui.text(
            t("You can change it any time in your profile (Account)."),
            r.x + 20.0,
            r.y + 122.0,
            12.0,
            MUTED,
            false,
        );
        let half = (w - 50.0) / 2.0;
        let mut changed = false;
        if ui.button(Rect::new(r.x + 20.0, r.y + r.h - 54.0, half, 38.0), t("No, thanks"), true) {
            self.email_offer = None;
            config.email_offer_declined = true;
            changed = true;
        }
        if ui.primary_button(
            Rect::new(r.x + 30.0 + half, r.y + r.h - 54.0, half, 38.0),
            t("Use this e-mail"),
        ) {
            self.send(ClientMsg::SetRecoveryEmail { email });
            self.email_offer = None;
        }
        changed
    }

    /// Short messages (achievements, declined invitations…), on top of everything.
    pub fn draw_toasts(&self, ui: &Ui, bottom: f32) {
        let sw = crate::ui::vw();
        let mut y = bottom;
        for (text, ttl, good) in self.toasts.iter().rev() {
            let alpha = ttl.min(1.0);
            let w = ui.text_size(text, 15.0, true).x + 40.0;
            y -= 40.0;
            let r = Rect::new((sw - w) / 2.0, y, w, 34.0);
            rrect(r, 17.0, Color::new(0.04, 0.03, 0.02, 0.88 * alpha));
            let edge = if *good { GOLD } else { ALERT };
            rrect_lines(r, 17.0, 1.2, Color::new(edge.r, edge.g, edge.b, 0.7 * alpha));
            let color = Color::new(CREAM.r, CREAM.g, CREAM.b, alpha);
            ui.text_centered(text, sw / 2.0, r.y + 9.0, 15.0, color, true);
        }
    }

    /// Invitations received at the table or in the lounge: answered from a banner.
    pub fn draw_invites(&mut self, ui: &mut Ui, in_match: bool) {
        let sw = crate::ui::vw();
        let mut y = if in_match { 64.0 } else { 74.0 };
        let mut answer = None;
        for (from, rated) in self.incoming.iter().take(3) {
            let w = 440.0f32.min(sw - 20.0);
            let r = Rect::new((sw - w) / 2.0, y, w, 64.0);
            ui.panel(r);
            rrect_lines(r, 8.0, 2.0, GOLD);
            let kind = if *rated { t("a rated game") } else { t("a casual game") };
            let color =
                self.players.iter().find(|p| &p.username == from).map(name_color).unwrap_or(CREAM);
            let country = self
                .players
                .iter()
                .find(|p| &p.username == from)
                .map(|p| p.country.clone())
                .unwrap_or_default();
            let used =
                ui.name_with_flag(from, &country, color, r.x + 14.0, r.y + 10.0, 16.0, w - 240.0);
            ui.text(
                &tw("invites you to {kind}", &[("kind", &kind)]),
                r.x + 20.0 + used,
                r.y + 11.0,
                14.0,
                CREAM,
                false,
            );
            if in_match {
                ui.text(t("Finish this match first."), r.x + 14.0, r.y + 38.0, 13.0, MUTED, false);
            } else if ui
                .primary_button(Rect::new(r.x + w - 214.0, r.y + 30.0, 96.0, 26.0), t("Accept"))
            {
                answer = Some((from.clone(), true));
            }
            if ui.button(Rect::new(r.x + w - 110.0, r.y + 30.0, 96.0, 26.0), t("Decline"), true) {
                answer = Some((from.clone(), false));
            }
            y += 70.0;
        }
        if let Some((from, accept)) = answer {
            self.incoming.retain(|(f, _)| *f != from);
            self.send(ClientMsg::AnswerInvite { from, accept });
        }
    }

    /// The sign-in form, a waiting message or the lounge.
    pub fn draw(&mut self, ui: &mut Ui, config: &mut Config) -> OnlineAction {
        match self.stage.clone() {
            Stage::Lounge => self.draw_lounge(ui, config),
            Stage::Reconnecting { attempt, .. } => {
                let text =
                    tw("Connection lost. Reconnecting (attempt {n})…", &[("n", &(attempt + 1))]);
                self.waiting(ui, &text)
            }
            Stage::Connecting | Stage::Idle => {
                let text = if self.lan {
                    t("Joining the LAN game…").to_string()
                } else {
                    t("Connecting to the server…").to_string()
                };
                let hint = (!self.lan).then(|| {
                    t("The first connection of the day can take up to a minute while the server wakes up.")
                });
                self.waiting_hint(ui, &text, hint)
            }
            Stage::Authenticating => self.waiting(ui, t("Signing in…")),
            Stage::SignIn => self.draw_sign_in(ui, config),
        }
    }

    fn waiting(&mut self, ui: &mut Ui, text: &str) -> OnlineAction {
        self.waiting_hint(ui, text, None)
    }

    fn waiting_hint(&mut self, ui: &mut Ui, text: &str, hint: Option<&str>) -> OnlineAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let r = Rect::new(sw / 2.0 - 240.0, sh / 2.0 - 80.0, 480.0, 160.0);
        ui.panel(r);
        ui.text_centered(text, sw / 2.0, r.y + 24.0, 17.0, CREAM, true);
        // A spinning ring of dots.
        let c = vec2(sw / 2.0, r.y + 66.0);
        let time = get_time() as f32;
        for k in 0..10 {
            let a = k as f32 / 10.0 * std::f32::consts::TAU + time * 4.0;
            let alpha = 0.15 + 0.85 * ((k as f32 / 10.0 + time * 0.0).fract());
            draw_circle(
                c.x + a.cos() * 14.0,
                c.y + a.sin() * 14.0,
                2.6,
                Color::new(GOLD.r, GOLD.g, GOLD.b, alpha),
            );
        }
        if let Some(h) = hint {
            for (k, line) in ui.wrap(h, r.w - 40.0, 12.0).iter().take(2).enumerate() {
                ui.text_centered(line, sw / 2.0, r.y + 90.0 + k as f32 * 15.0, 12.0, MUTED, false);
            }
        }
        if ui.button(Rect::new(sw / 2.0 - 70.0, r.y + r.h - 40.0, 140.0, 30.0), t("Back"), true) {
            if !self.signed_in() {
                self.conn = None;
                self.queue = None;
                self.stage = Stage::Idle;
            }
            return OnlineAction::Leave;
        }
        OnlineAction::None
    }

    fn draw_sign_in(&mut self, ui: &mut Ui, config: &mut Config) -> OnlineAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        let w = 440.0f32.min(sw - 20.0);
        let h = if self.register { 490.0 } else { 400.0 };
        let r = Rect::new((sw - w) / 2.0, ((sh - h) / 2.0).max(10.0), w, h);
        ui.panel(r);
        let x = r.x + 28.0;
        let inner = w - 56.0;
        let mut y = r.y + 20.0;
        ui.text_centered(
            if self.register { t("Create your account") } else { t("Sign in") },
            sw / 2.0,
            y,
            24.0,
            GOLD,
            true,
        );
        y += 38.0;
        let tabs = [t("Sign in"), t("New account")];
        if let Some(i) = ui.segmented(Rect::new(x, y, inner, 32.0), &tabs, self.register as usize) {
            self.register = i == 1;
            self.error = None;
        }
        y += 50.0;
        let mut submit = false;
        ui.text(t("Username"), x, y, 13.0, MUTED, true);
        submit |= ui.text_input(20, Rect::new(x, y + 18.0, inner, 34.0), &mut self.username, 16);
        y += 62.0;
        ui.text(t("Password"), x, y, 13.0, MUTED, true);
        submit |=
            ui.password_input(21, Rect::new(x, y + 18.0, inner, 34.0), &mut self.password, 72);
        y += 62.0;
        if self.register {
            ui.text(t("Country"), x, y, 13.0, MUTED, true);
            let label = if self.country.is_empty() {
                t("Choose…").to_string()
            } else {
                crate::flags::name(&self.country).to_string()
            };
            let cr = Rect::new(x, y + 18.0, inner, 34.0);
            if crate::countries::field(ui, cr, &self.country, &label) {
                crate::countries::open();
            }
            y += 56.0;
            let hint = t("At least 8 characters. Use a password you don't use elsewhere.");
            for line in ui.wrap(hint, inner, 12.0) {
                ui.text(&line, x, y, 12.0, MUTED, false);
                y += 15.0;
            }
            y += 6.0;
        }

        let name_ok = valid_username(self.username.trim());
        let pass_ok = valid_password(&self.password);
        let connected = self.conn.is_some();
        let label = if self.register { t("Create account") } else { t("Sign in") };
        if ui.primary_button(Rect::new(x, y, inner, 42.0), label) || (submit && name_ok && pass_ok)
        {
            if !connected {
                self.connect();
                self.error = None;
            } else if !name_ok {
                self.error = Some(t("Usernames have 3–16 letters, digits or underscores.").into());
            } else if !pass_ok {
                self.error =
                    Some(t("Passwords need at least 8 characters (not only digits).").into());
            } else {
                let username = self.username.trim().to_string();
                let password = self.password.clone();
                self.send(if self.register {
                    ClientMsg::Register { username, password, country: self.country.clone() }
                } else {
                    ClientMsg::Login { username, password }
                });
                self.auto_resume = false;
                self.error = None;
                self.stage = Stage::Authenticating;
            }
        }
        y += 54.0;
        match &self.error {
            Some(e) => {
                for (k, line) in ui.wrap(e, inner, 14.0).iter().take(3).enumerate() {
                    ui.text_centered(line, sw / 2.0, y + k as f32 * 18.0, 14.0, ALERT, false);
                }
            }
            None if !self.register => ui.text_centered(
                t("New here? Choose “New account”: it takes ten seconds."),
                sw / 2.0,
                y,
                13.0,
                MUTED,
                false,
            ),
            None => {}
        }
        if let Some(code) = crate::countries::draw_picker(ui) {
            self.country = code;
        }
        if !connected
            && ui.button(
                Rect::new(x + inner - 110.0, r.y + r.h - 44.0, 110.0, 30.0),
                t("Retry"),
                true,
            )
        {
            self.server = config.server.clone();
            self.connect();
            self.error = None;
        }
        if ui.button(Rect::new(x, r.y + r.h - 44.0, 110.0, 30.0), t("Back"), true) {
            return OnlineAction::Leave;
        }
        OnlineAction::None
    }

    fn draw_lounge(&mut self, ui: &mut Ui, config: &mut Config) -> OnlineAction {
        let sw = crate::ui::vw();
        let sh = crate::ui::vh();
        // A dark wash over the slowly turning table: the room is the backdrop.
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.42));
        let mut action = OnlineAction::None;
        let margin = 14.0;

        // Header.
        let header = Rect::new(margin, margin, sw - margin * 2.0, 52.0);
        ui.panel(header);
        if ui.icon_button(
            Rect::new(header.x + 10.0, header.y + 9.0, 34.0, 34.0),
            Icon::Back,
            t("Main menu"),
            true,
        ) {
            action = OnlineAction::Leave;
        }
        crate::icon::draw(header.x + 52.0, header.y + 8.0, 36.0);
        let name = snookered_proto::GAME_NAME.to_uppercase();
        ui.text(&name, header.x + 96.0, header.y + 12.0, 24.0, GOLD, true);
        let title_w = ui.text_size(&name, 24.0, true).x;
        let title = if self.lan { t("LAN game") } else { t("lounge") };
        ui.text(title, header.x + 106.0 + title_w, header.y + 19.0, 15.0, MUTED, false);
        let online_text = match self.rtt.filter(|_| config.show_ping) {
            Some(rtt) => tw(
                "{n} online · ping {ms} ms",
                &[("n", &self.players.len()), ("ms", &rtt.as_millis())],
            ),
            None => tw("{n} online", &[("n", &self.players.len())]),
        };
        let mut right = header.x + header.w - 10.0;
        if !self.lan {
            right -= 34.0;
            if ui.icon_button(
                Rect::new(right, header.y + 9.0, 34.0, 34.0),
                Icon::Power,
                t("Sign out"),
                true,
            ) {
                self.logout(config);
                return OnlineAction::Leave;
            }
            right -= 40.0;
            if ui.icon_button(
                Rect::new(right, header.y + 9.0, 34.0, 34.0),
                Icon::Trophy,
                t("World ranking"),
                true,
            ) {
                action = OnlineAction::Ranking;
            }
            // The coins, which open the shop.
            let coins = self.wallet.as_ref().map(|w| w.coins).unwrap_or(0).to_string();
            let cw = ui.text_size(&coins, 14.0, true).x + 44.0;
            right -= cw + 6.0;
            let chip = Rect::new(right, header.y + 9.0, cw, 34.0);
            let hover = ui.hovering(chip);
            ui.block(chip);
            rrect(chip, 8.0, Color::new(1.0, 1.0, 1.0, if hover { 0.10 } else { 0.04 }));
            rrect_lines(chip, 8.0, 1.2, if hover { GOLD } else { BORDER });
            crate::icons::draw(Icon::Coin, vec2(chip.x + 16.0, chip.y + 17.0), 18.0, GOLD);
            ui.text(&coins, chip.x + 30.0, chip.y + 9.0, 14.0, CREAM, true);
            if hover {
                ui.tooltip(t("Shop"), chip);
            }
            if ui.clicked(chip) {
                action = OnlineAction::Shop;
            }
        }
        // Who am I.
        if let Some(me) = self.me.clone() {
            let card_w = 250.0f32.min(right - header.x - 300.0).max(120.0);
            let card = Rect::new(right - card_w - 10.0, header.y + 6.0, card_w, 40.0);
            let hover = ui.hovering(card) && !self.lan;
            rrect(card, 8.0, Color::new(1.0, 1.0, 1.0, if hover { 0.08 } else { 0.03 }));
            ui.player_name(&me, card.x + 10.0, card.y + 4.0, 15.0, card.w - 20.0);
            let sub = if me.guest {
                t("guest").to_string()
            } else {
                format!("{} · {}", me.rating, t(me.tier().name()))
            };
            ui.text(&sub, card.x + 10.0, card.y + 22.0, 12.0, MUTED, false);
            ui.block(card);
            if hover && ui.clicked(card) {
                action = OnlineAction::Profile(me.username.clone());
            }
            right = card.x;
        }
        let ow = ui.text_size(&online_text, 13.0, false).x;
        draw_circle(right - ow - 22.0, header.y + 26.0, 4.0, Color::new(0.35, 0.85, 0.45, 1.0));
        ui.text(&online_text, right - ow - 12.0, header.y + 19.0, 13.0, MUTED, false);

        let body_y = header.y + header.h + 10.0;
        let body_h = sh - body_y - margin;
        let left_w = (sw * 0.46).clamp(330.0, 560.0);
        let left = Rect::new(margin, body_y, left_w, body_h);
        let right_panel =
            Rect::new(margin + left_w + 10.0, body_y, sw - left_w - margin * 2.0 - 10.0, body_h);

        // Left: tabs.
        ui.panel(left);
        let mut tabs = vec![
            (Tab::Players, t("Players")),
            (Tab::Tables, t("Tables")),
            (Tab::Tournaments, t("Tournaments")),
        ];
        if !self.lan {
            tabs.push((Tab::Friends, t("Friends")));
        }
        let tab_w = (left.w - 20.0 - (tabs.len() - 1) as f32 * 6.0) / tabs.len() as f32;
        for (k, (tab, label)) in tabs.into_iter().enumerate() {
            let r = Rect::new(left.x + 10.0 + k as f32 * (tab_w + 6.0), left.y + 10.0, tab_w, 32.0);
            let label = if tab == Tab::Tables && !self.tables.is_empty() {
                format!("{label} ({})", self.tables.len())
            } else {
                label.to_string()
            };
            if ui.tab(r, &label, self.tab == tab) {
                self.tab = tab;
                self.list_first = 0;
                if tab == Tab::Friends {
                    self.send(ClientMsg::Friends);
                    self.unread_friend_requests = false;
                }
            }
            let badge = match tab {
                Tab::Friends => self.unread_friend_requests || !self.friends.incoming.is_empty(),
                _ => false,
            };
            if badge {
                draw_circle(r.x + r.w - 8.0, r.y + 7.0, 4.5, ALERT);
            }
        }
        let reserve = if self.outgoing.is_some() { 60.0 } else { 0.0 };
        let area = Rect::new(left.x + 12.0, left.y + 54.0, left.w - 24.0, left.h - 64.0 - reserve);
        match self.tab {
            Tab::Players => {
                if let Some(a) = self.draw_players(ui, area) {
                    action = a;
                }
            }
            Tab::Tables => self.draw_tables(ui, area),
            Tab::Tournaments => self.draw_tournaments(ui, area),
            Tab::Friends => {
                if let Some(a) = self.draw_friends(ui, area) {
                    action = a;
                }
            }
        }

        // Outgoing invitation.
        if let Some((to, rated)) = self.outgoing.clone() {
            let r = Rect::new(left.x + 10.0, left.y + left.h - 58.0, left.w - 20.0, 48.0);
            rrect(r, 8.0, Color::new(0.18, 0.12, 0.06, 0.95));
            rrect_lines(r, 8.0, 1.5, GOLD);
            let kind = if rated { t("rated") } else { t("casual") };
            let text = ui.ellipsize(
                &tw("Waiting for {to} ({kind})…", &[("to", &to), ("kind", &kind)]),
                r.w - 120.0,
                15.0,
            );
            ui.text(&text, r.x + 14.0, r.y + 15.0, 15.0, CREAM, false);
            if ui.button(Rect::new(r.x + r.w - 98.0, r.y + 10.0, 88.0, 28.0), t("Cancel"), true) {
                self.send(ClientMsg::CancelInvite { to });
                self.outgoing = None;
            }
        }

        // Right: the chat, the missions and the board.
        ui.panel(right_panel);
        let mut log_top = right_panel.y + 40.0;
        if self.lan {
            ui.text(t("Lounge chat"), right_panel.x + 14.0, right_panel.y + 12.0, 16.0, GOLD, true);
        } else {
            self.side_tabs(ui, right_panel, config);
            log_top = right_panel.y + 50.0;
        }
        if !self.motd.is_empty() {
            let lines = ui.wrap(&self.motd, right_panel.w - 40.0, 13.0);
            let mh = lines.len().min(2) as f32 * 16.0 + 12.0;
            let mr = Rect::new(right_panel.x + 12.0, log_top, right_panel.w - 24.0, mh);
            rrect(mr, 6.0, Color::new(0.91, 0.75, 0.38, 0.08));
            for (k, l) in lines.iter().take(2).enumerate() {
                ui.text(
                    l,
                    mr.x + 10.0,
                    mr.y + 6.0 + k as f32 * 16.0,
                    13.0,
                    Color::new(1.0, 0.9, 0.7, 0.9),
                    false,
                );
            }
            log_top += mh + 8.0;
        }
        // The server is about to restart.
        if let Some(until) = self.draining {
            let left = until.saturating_duration_since(std::time::Instant::now()).as_secs();
            let text = tw(
                "The server restarts in {m}:{s} for an update. Games in progress go on.",
                &[("m", &(left / 60)), ("s", &format!("{:02}", left % 60))],
            );
            let lines = ui.wrap(&text, right_panel.w - 44.0, 13.0);
            let r = Rect::new(
                right_panel.x + 12.0,
                log_top,
                right_panel.w - 24.0,
                lines.len() as f32 * 16.0 + 14.0,
            );
            rrect(r, 6.0, Color::new(1.0, 0.56, 0.44, 0.14));
            rrect_lines(r, 6.0, 1.0, ALERT);
            for (k, l) in lines.iter().enumerate() {
                ui.text(l, r.x + 10.0, r.y + 7.0 + k as f32 * 16.0, 13.0, CREAM, true);
            }
            log_top += r.h + 8.0;
        }
        if self.side != Side::Chat {
            let area = Rect::new(
                right_panel.x + 12.0,
                log_top,
                right_panel.w - 24.0,
                right_panel.y + right_panel.h - log_top - 12.0,
            );
            match self.side {
                Side::Missions => self.draw_missions(ui, area),
                _ => self.draw_board(ui, area),
            }
            self.draw_invites(ui, false);
            return action;
        }
        // The featured game: anyone can watch it.
        if let Some(f) = self.featured.clone()
            && !self.lan
        {
            let r = Rect::new(right_panel.x + 12.0, log_top, right_panel.w - 24.0, 52.0);
            rrect(r, 8.0, Color::new(0.0, 0.0, 0.0, 0.3));
            rrect_lines(r, 8.0, 1.0, BORDER);
            let live = 0.5 + 0.5 * (get_time() as f32 * 2.5).sin();
            draw_circle(r.x + 14.0, r.y + 14.0, 4.0, Color::new(1.0, 0.3, 0.25, 0.5 + 0.5 * live));
            let on_tv = t("ON TV");
            ui.text(on_tv, r.x + 24.0, r.y + 7.0, 11.0, ALERT, true);
            let avg = (f.players[0].rating + f.players[1].rating) / 2;
            let sub = tw("{n} watching · avg {r}", &[("n", &f.spectators), ("r", &avg)]);
            // After the label, however long it is in this language ("EN LA TELE").
            let sub_x = r.x + 24.0 + ui.text_size(on_tv, 11.0, true).x + 12.0;
            ui.text(&sub, sub_x, r.y + 7.0, 11.0, MUTED, false);
            let half = (r.w - 110.0) / 2.0;
            ui.player_name(&f.players[0], r.x + 12.0, r.y + 26.0, 14.0, half - 14.0);
            ui.text(t("vs"), r.x + 12.0 + half - 8.0, r.y + 27.0, 12.0, MUTED, false);
            ui.player_name(&f.players[1], r.x + 22.0 + half, r.y + 26.0, 14.0, half - 14.0);
            let mine = self
                .me
                .as_ref()
                .is_some_and(|m| f.players.iter().any(|p| p.username == m.username));
            if !mine
                && ui.button(Rect::new(r.x + r.w - 82.0, r.y + 12.0, 72.0, 28.0), t("Watch"), true)
            {
                self.send(ClientMsg::WatchMatch { match_id: f.match_id });
            }
            log_top += r.h + 8.0;
        }
        let input_h = 36.0;
        let log = Rect::new(
            right_panel.x + 12.0,
            log_top,
            right_panel.w - 24.0,
            right_panel.y + right_panel.h - log_top - input_h - 22.0,
        );
        rrect(log, 6.0, Color::new(0.0, 0.0, 0.0, 0.25));
        let me = self.me.as_ref().map(|m| m.username.clone());
        self.chat_scroll = ui.chat_log(log, &self.chat, me.as_deref(), self.chat_scroll);
        let input = Rect::new(
            right_panel.x + 12.0,
            right_panel.y + right_panel.h - input_h - 12.0,
            right_panel.w - 24.0 - 48.0,
            input_h,
        );
        let sent = ui.text_input(30, input, &mut self.chat_input, MAX_CHAT_LEN);
        if self.chat_input.is_empty() && ui.focus != Some(30) {
            ui.text(
                t("Say hello… (Enter to send)"),
                input.x + 10.0,
                input.y + 10.0,
                15.0,
                MUTED,
                false,
            );
        }
        let send_r = Rect::new(input.x + input.w + 8.0, input.y, 40.0, input_h);
        let can_send = !self.chat_input.trim().is_empty();
        if (ui.icon_button(send_r, Icon::Play, t("Send"), can_send) || sent) && can_send {
            let text = snookered_proto::clean_chat(&self.chat_input);
            self.send(ClientMsg::Chat { text });
            self.chat_input.clear();
            self.chat_scroll = 0;
            ui.focus = Some(30);
        }

        self.draw_invites(ui, false);
        action
    }

    /// A scheduled tournament (the weekly cup) starts within ten minutes:
    /// tell the player once, whether they signed up or not.
    fn tournaments_starting(&mut self) {
        if !self.signed_in() || self.lan {
            return;
        }
        let now = crate::game::unix_now();
        let me = self.me.as_ref().map(|m| m.username.clone()).unwrap_or_default();
        let soon: Vec<(u64, String, bool)> = self
            .tournaments
            .iter()
            .filter(|t| t.state == TournamentState::Registering && !t.private)
            .filter(|t| t.starts_at.is_some_and(|at| at > now && at - now <= 600))
            .filter(|t| !self.told_starting.contains(&t.id))
            .map(|t| (t.id, t.name.clone(), t.players.iter().any(|p| p.username == me)))
            .collect();
        for (id, name, signed_up) in soon {
            self.told_starting.push(id);
            let text = if signed_up {
                tw("{name} starts in a few minutes: get ready!", &[("name", &name)])
            } else if self
                .tournaments
                .iter()
                .any(|t| t.id == id && t.players.len() < t.size as usize)
            {
                tw("{name} starts in a few minutes: there is still room.", &[("name", &name)])
            } else {
                continue;
            };
            self.toast(text.clone(), true);
            self.attention = Some(text);
        }
    }

    /// The tabs over the right side of the lounge.
    fn side_tabs(&mut self, ui: &mut Ui, panel: Rect, config: &mut Config) {
        let tabs =
            [(Side::Chat, t("Chat")), (Side::Missions, t("Missions")), (Side::Board, t("Board"))];
        let w = (panel.w - 24.0 - 12.0) / 3.0;
        let newest = self.board.iter().map(|n| n.id).max().unwrap_or(0);
        // Notes that arrive while the board is open are seen.
        if self.side == Side::Board && newest > config.board_seen {
            config.board_seen = newest;
            config.save();
        }
        for (k, (side, label)) in tabs.into_iter().enumerate() {
            let r = Rect::new(panel.x + 12.0 + k as f32 * (w + 6.0), panel.y + 10.0, w, 32.0);
            if ui.tab(r, label, self.side == side) {
                self.side = side;
                match side {
                    Side::Missions => {
                        self.mission_done = false;
                        self.send(ClientMsg::Missions);
                    }
                    Side::Board if newest > config.board_seen => {
                        config.board_seen = newest;
                        config.save();
                    }
                    _ => {}
                }
            }
            let badge = match side {
                Side::Missions => self.mission_done,
                Side::Board => newest > config.board_seen,
                Side::Chat => false,
            };
            if badge {
                draw_circle(r.x + r.w - 8.0, r.y + 7.0, 4.5, ALERT);
            }
        }
    }

    /// Today's and this week's missions with the player's progress.
    fn draw_missions(&mut self, ui: &mut Ui, area: Rect) {
        let Some(v) = &self.missions else {
            ui.text(t("Loading…"), area.x, area.y + 6.0, 14.0, MUTED, false);
            return;
        };
        let now = crate::game::unix_now();
        let mut y = area.y + 2.0;
        let sections = [
            (
                t("Today"),
                tw("new missions in {t}", &[("t", &time_left(v.day_ends - now))]),
                &v.daily,
            ),
            (t("This week"), tw("ends in {t}", &[("t", &time_left(v.week_ends - now))]), &v.weekly),
        ];
        for (title, when, list) in sections {
            ui.text(title, area.x, y, 16.0, GOLD, true);
            let tw_ = ui.text_size(title, 16.0, true).x;
            ui.text(&when, area.x + tw_ + 10.0, y + 3.0, 12.0, MUTED, false);
            y += 26.0;
            for state in list {
                let Some(m) = snookered_proto::missions::mission(&state.code) else { continue };
                let card = Rect::new(area.x, y, area.w, 56.0);
                let done = state.progress >= m.target;
                rrect(card, 8.0, Color::new(1.0, 1.0, 1.0, if done { 0.07 } else { 0.035 }));
                rrect_lines(card, 8.0, 1.0, if done { GOLD } else { BORDER });
                let text = tw(m.text, &[("n", &m.target)]);
                let text = ui.ellipsize(&text, card.w - 120.0, 14.0);
                ui.text(&text, card.x + 12.0, card.y + 9.0, 14.0, CREAM, true);
                // Progress.
                let bar = Rect::new(card.x + 12.0, card.y + 36.0, card.w - 130.0, 7.0);
                rrect(bar, 3.5, Color::new(0.0, 0.0, 0.0, 0.4));
                let k = (state.progress as f32 / m.target.max(1) as f32).clamp(0.0, 1.0);
                if k > 0.0 {
                    let fill = Rect::new(bar.x, bar.y, (bar.w * k).max(7.0), bar.h);
                    rrect(fill, 3.5, if done { GOLD } else { Color::new(0.35, 0.75, 0.5, 1.0) });
                }
                let count = format!("{}/{}", state.progress.min(m.target), m.target);
                ui.text(&count, bar.x + bar.w + 8.0, card.y + 31.0, 12.0, MUTED, false);
                // The reward.
                let reward = m.reward.to_string();
                let rw = ui.text_size(&reward, 15.0, true).x;
                let rx = card.x + card.w - rw - 14.0;
                if done {
                    crate::icons::draw(Icon::Check, vec2(rx - 16.0, card.y + 28.0), 18.0, GOLD);
                } else {
                    crate::icons::draw(Icon::Coin, vec2(rx - 14.0, card.y + 28.0), 18.0, GOLD);
                }
                ui.text(&reward, rx, card.y + 19.0, 15.0, if done { GOLD } else { CREAM }, true);
                y += card.h + 8.0;
            }
            y += 10.0;
        }
        let note = t(
            "Missions count rated games played to the end. The coins arrive as soon as one is complete.",
        );
        for (i, line) in ui.wrap(note, area.w, 12.0).iter().enumerate() {
            if y + i as f32 * 16.0 > area.y + area.h - 14.0 {
                break;
            }
            ui.text(line, area.x, y + i as f32 * 16.0, 12.0, MUTED, false);
        }
    }

    /// The staff's notes and the tournaments open for registration.
    fn draw_board(&mut self, ui: &mut Ui, area: Rect) {
        let now = crate::game::unix_now();
        let bottom = area.y + area.h;
        let mut y = area.y + 2.0;
        let notes: Vec<_> =
            self.board.iter().filter(|n| n.expires_at.is_none_or(|e| e > now)).cloned().collect();
        let open: Vec<TournamentInfo> = self
            .tournaments
            .iter()
            .filter(|t| t.state == TournamentState::Registering && !t.private)
            .cloned()
            .collect();
        if notes.is_empty() && open.is_empty() {
            ui.text(t("Nothing on the board right now."), area.x, y + 4.0, 14.0, MUTED, false);
            return;
        }
        for n in &notes {
            let lines = ui.wrap(&n.text, area.w - 44.0, 14.0);
            let h = lines.len() as f32 * 19.0 + 38.0;
            if y + h > bottom {
                break;
            }
            let card = Rect::new(area.x, y, area.w, h);
            rrect(card, 8.0, Color::new(0.91, 0.75, 0.38, 0.07));
            rrect_lines(card, 8.0, 1.0, BORDER);
            draw_circle(card.x + card.w - 14.0, card.y + 12.0, 4.0, GOLD);
            for (i, l) in lines.iter().enumerate() {
                ui.text(l, card.x + 12.0, card.y + 10.0 + i as f32 * 19.0, 14.0, CREAM, false);
            }
            let by = if n.author.is_empty() {
                crate::profile::date(n.at)
            } else {
                format!("{} · {}", n.author, crate::profile::date(n.at))
            };
            ui.text(&by, card.x + 12.0, card.y + h - 22.0, 12.0, MUTED, false);
            y += h + 8.0;
        }
        if open.is_empty() || y + 60.0 > bottom {
            return;
        }
        y += 6.0;
        ui.text(t("Open tournaments"), area.x, y, 16.0, GOLD, true);
        y += 26.0;
        for tour in open {
            if y + 50.0 > bottom {
                break;
            }
            let card = Rect::new(area.x, y, area.w, 50.0);
            rrect(card, 8.0, Color::new(1.0, 1.0, 1.0, 0.035));
            rrect_lines(card, 8.0, 1.0, BORDER);
            crate::icons::draw(Icon::Trophy, vec2(card.x + 20.0, card.y + 25.0), 20.0, GOLD);
            let name = ui.ellipsize(&tour.name, card.w - 140.0, 14.0);
            ui.text(&name, card.x + 40.0, card.y + 8.0, 14.0, CREAM, true);
            let mut sub =
                tw("{n}/{size} players", &[("n", &tour.players.len()), ("size", &tour.size)]);
            if let Some(at) = tour.starts_at.filter(|&a| a > now) {
                sub = format!("{sub} · {}", tw("starts in {t}", &[("t", &time_left(at - now))]));
            }
            if tour.entry_fee > 0 {
                sub = format!("{sub} · {}", tw("entry {n} coins", &[("n", &tour.entry_fee)]));
            }
            let sub = ui.ellipsize(&sub, card.w - 140.0, 12.0);
            ui.text(&sub, card.x + 40.0, card.y + 28.0, 12.0, MUTED, false);
            let b = Rect::new(card.x + card.w - 90.0, card.y + 10.0, 80.0, 30.0);
            if ui.button(b, t("See"), true) {
                self.tab = Tab::Tournaments;
                self.selected_tournament = Some(tour.id);
            }
            y += card.h + 8.0;
        }
    }

    fn rated_toggle(&mut self, ui: &mut Ui, r: Rect) {
        if self.lan {
            ui.text(t("LAN games don't change ratings."), r.x, r.y + 8.0, 13.0, MUTED, false);
            return;
        }
        let options = [t("Rated"), t("Casual")];
        if let Some(i) = ui.segmented(r, &options, if self.rated_games { 0 } else { 1 }) {
            self.rated_games = i == 0;
        }
    }

    fn status_text(p: &PlayerInfo) -> String {
        match p.status {
            PlayerStatus::Playing => t("playing").to_string(),
            PlayerStatus::Waiting => t("at a table").to_string(),
            PlayerStatus::Available => {
                if p.guest {
                    t("guest").to_string()
                } else {
                    format!("{} · {}", p.rating, t(p.tier().name()))
                }
            }
        }
    }

    fn status_dot(p: &PlayerInfo, x: f32, y: f32) {
        let color = match p.status {
            PlayerStatus::Playing => Color::new(0.9, 0.6, 0.2, 1.0),
            PlayerStatus::Waiting => Color::new(0.45, 0.7, 1.0, 1.0),
            PlayerStatus::Available => Color::new(0.35, 0.85, 0.45, 1.0),
        };
        draw_circle(x, y, 4.5, color);
    }

    /// "Play now": one click to be paired with someone of a similar rating.
    fn draw_play_now(&mut self, ui: &mut Ui, r: Rect) {
        match self.queue {
            None => {
                if ui.primary_button(r, t("Play now")) {
                    self.send(ClientMsg::QueueJoin { rated: self.rated_games && !self.lan });
                }
                if ui.hovering(r) {
                    ui.tooltip(t("Find an opponent of your level automatically"), r);
                }
            }
            Some((rated, waited, range, at)) => {
                rrect(r, 8.0, Color::new(0.10, 0.30, 0.18, 0.55));
                rrect_lines(r, 8.0, 1.5, Color::new(0.35, 0.8, 0.5, 0.8));
                // A small spinner.
                let c = vec2(r.x + 22.0, r.y + r.h / 2.0);
                let a = get_time() as f32 * 5.0;
                for k in 0..8 {
                    let ang = a + k as f32 * std::f32::consts::TAU / 8.0;
                    let p = c + Vec2::from_angle(ang) * 8.0;
                    draw_circle(p.x, p.y, 1.8, Color::new(0.6, 1.0, 0.7, (k as f32 + 1.0) / 8.0));
                }
                let secs = waited + at.elapsed().as_secs() as u32;
                let title = tw(
                    "Looking for an opponent… {t}",
                    &[("t", &format!("{}:{:02}", secs / 60, secs % 60))],
                );
                let sub = if range == u32::MAX {
                    t("any rating").to_string()
                } else {
                    tw(
                        "rating ±{n} · {mode}",
                        &[("n", &range), ("mode", &if rated { t("rated") } else { t("casual") })],
                    )
                };
                let cw = 100.0f32.min(r.w * 0.3);
                let (title, size) = ui.fitted(&title, 14.0, r.w - cw - 56.0, true);
                ui.text(&title, r.x + 40.0, r.y + 7.0, size, CREAM, true);
                ui.text(&sub, r.x + 40.0, r.y + r.h - 19.0, 12.0, MUTED, false);
                if ui.button(
                    Rect::new(r.x + r.w - cw - 8.0, r.y + 7.0, cw, r.h - 14.0),
                    t("Cancel"),
                    true,
                ) {
                    self.send(ClientMsg::QueueLeave);
                    self.queue = None;
                }
            }
        }
    }

    fn draw_players(&mut self, ui: &mut Ui, area: Rect) -> Option<OnlineAction> {
        self.draw_play_now(ui, Rect::new(area.x, area.y, area.w, 44.0));
        let area = Rect::new(area.x, area.y + 54.0, area.w, area.h - 54.0);
        self.rated_toggle(ui, Rect::new(area.x, area.y, area.w.min(260.0), 30.0));
        let me = self.me.as_ref().map(|m| m.username.clone()).unwrap_or_default();
        let mut players: Vec<PlayerInfo> =
            self.players.iter().filter(|p| p.username != me).cloned().collect();
        players.sort_by(|a, b| {
            (a.status != PlayerStatus::Available)
                .cmp(&(b.status != PlayerStatus::Available))
                .then(b.rating.cmp(&a.rating))
        });
        let list = Rect::new(area.x, area.y + 42.0, area.w, area.h - 42.0);
        if players.is_empty() {
            ui.text(t("Nobody else is here right now."), list.x, list.y + 10.0, 15.0, MUTED, false);
            ui.text(
                t("Open a table and wait, or invite a friend!"),
                list.x,
                list.y + 32.0,
                14.0,
                MUTED,
                false,
            );
            let b = Rect::new(list.x, list.y + 62.0, 200.0f32.min(list.w), 36.0);
            if ui.primary_button(b, t("Open a table")) {
                self.tab = Tab::Tables;
            }
            return None;
        }
        let row_h = 46.0;
        let visible = (list.h / row_h).floor().max(1.0) as usize;
        self.list_first = ui.scroll_list(list, self.list_first, players.len(), visible);
        let mut invite = None;
        let mut open = None;
        let mut watch = None;
        for (k, p) in players.iter().skip(self.list_first).take(visible).enumerate() {
            let r = Rect::new(list.x, list.y + k as f32 * row_h, list.w - 8.0, row_h - 6.0);
            let hover = ui.hovering(r);
            rrect(r, 7.0, Color::new(1.0, 1.0, 1.0, if hover { 0.07 } else { 0.035 }));
            Self::status_dot(p, r.x + 12.0, r.y + r.h / 2.0);
            let name_r = Rect::new(r.x + 24.0, r.y + 3.0, r.w - 140.0, r.h - 6.0);
            ui.player_name(p, name_r.x, r.y + 5.0, 15.0, name_r.w);
            ui.text(&Self::status_text(p), name_r.x, r.y + 23.0, 12.0, MUTED, false);
            if ui.clicked(name_r) && !p.guest {
                open = Some(p.username.clone());
            }
            let button = Rect::new(r.x + r.w - 96.0, r.y + 7.0, 88.0, r.h - 14.0);
            // Friends' games (anyone's on a local network) can be watched.
            let friend =
                self.lan || self.friends.friends.iter().any(|f| f.player.username == p.username);
            if p.status == PlayerStatus::Playing && friend {
                if ui.icon_button(button, Icon::Eye, t("Watch the game"), true) {
                    watch = Some(p.username.clone());
                }
            } else {
                let can = p.status == PlayerStatus::Available && self.outgoing.is_none();
                if ui.button(button, t("Invite"), can) {
                    invite = Some(p.username.clone());
                }
            }
        }
        if let Some(to) = invite {
            self.send(ClientMsg::Invite { to, rated: self.rated_games && !self.lan });
        }
        if let Some(name) = watch {
            self.spectate(&name);
        }
        open.map(OnlineAction::Profile)
    }

    fn draw_tables(&mut self, ui: &mut Ui, area: Rect) {
        let me = self.me.as_ref().map(|m| m.username.clone()).unwrap_or_default();
        let mine = self.tables.iter().find(|t| t.host.username == me).cloned();
        let mut y = area.y;
        match mine {
            Some(table) => {
                let r = Rect::new(area.x, y, area.w, 74.0);
                rrect(r, 8.0, Color::new(0.10, 0.30, 0.18, 0.55));
                rrect_lines(r, 8.0, 1.5, Color::new(0.35, 0.8, 0.5, 0.8));
                let pulse = 0.5 + 0.5 * (get_time() as f32 * 3.0).sin();
                draw_circle(
                    r.x + 18.0,
                    r.y + 22.0,
                    5.0,
                    Color::new(0.35, 0.95, 0.5, 0.4 + 0.6 * pulse),
                );
                ui.text(t("Your table is open"), r.x + 32.0, r.y + 12.0, 16.0, CREAM, true);
                let kind = match (table.rated, table.private) {
                    (true, true) => t("rated · private"),
                    (true, false) => t("rated · public"),
                    (false, true) => t("casual · private"),
                    (false, false) => t("casual · public"),
                };
                ui.text(
                    &format!("{kind} — {}", t("waiting for an opponent…")),
                    r.x + 32.0,
                    r.y + 36.0,
                    13.0,
                    MUTED,
                    false,
                );
                if ui.button(
                    Rect::new(r.x + r.w - 110.0, r.y + 22.0, 100.0, 30.0),
                    t("Close"),
                    true,
                ) {
                    self.send(ClientMsg::CloseTable);
                }
                y += 86.0;
            }
            None => {
                self.rated_toggle(ui, Rect::new(area.x, y, 220.0f32.min(area.w * 0.5), 30.0));
                let px = area.x + 232.0f32.min(area.w * 0.5 + 12.0);
                ui.checkbox(px, y + 6.0, t("Private"), &mut self.table_private);
                y += 40.0;
                if self.table_private {
                    ui.text(t("Password"), area.x, y + 9.0, 13.0, MUTED, true);
                    ui.text_input(
                        31,
                        Rect::new(area.x + 80.0, y, area.w - 80.0, 32.0),
                        &mut self.table_password,
                        32,
                    );
                    y += 40.0;
                }
                let can = !self.table_private || !self.table_password.trim().is_empty();
                if ui.button(Rect::new(area.x, y, area.w, 36.0), t("Open a table"), can) {
                    let password = self.table_private.then(|| self.table_password.clone());
                    self.send(ClientMsg::OpenTable {
                        rated: self.rated_games && !self.lan,
                        password,
                    });
                }
                y += 44.0;
                if !self.lan {
                    // Invite someone who isn't in the lounge yet: a short code.
                    let half = (area.w - 8.0) / 2.0;
                    if ui.button(Rect::new(area.x, y, half, 32.0), t("Invite with a code"), true) {
                        self.send(ClientMsg::CreateInviteCode { rated: self.rated_games });
                    }
                    let input = Rect::new(area.x + half + 8.0, y, half - 70.0, 32.0);
                    let enter = ui.text_input(37, input, &mut self.code_input, 24);
                    if self.code_input.is_empty() && ui.focus != Some(37) {
                        ui.text(t("Code"), input.x + 9.0, input.y + 9.0, 13.0, MUTED, false);
                    }
                    let ok = self.code_input.trim().len() >= 6;
                    if (ui.button(Rect::new(input.x + input.w + 6.0, y, 64.0, 32.0), t("Join"), ok)
                        || enter)
                        && ok
                    {
                        let code = std::mem::take(&mut self.code_input);
                        self.send(ClientMsg::JoinByCode { code });
                    }
                    y += 42.0;
                }
            }
        }
        // The code of this player's table, to send to a friend.
        if let Some(code) =
            self.invite_code.clone().filter(|_| self.tables.iter().any(|t| t.host.username == me))
        {
            let r = Rect::new(area.x, y, area.w, 48.0);
            rrect(r, 8.0, Color::new(0.91, 0.75, 0.38, 0.12));
            ui.text(&code, r.x + 14.0, r.y + 10.0, 24.0, GOLD, true);
            let link = format!("snookered://join/{code}");
            let cw = ui.text_size(&code, 24.0, true).x;
            ui.text(&link, r.x + 30.0 + cw, r.y + 17.0, 12.0, MUTED, false);
            if ui.button(Rect::new(r.x + r.w - 92.0, r.y + 9.0, 82.0, 30.0), t("Copy"), true) {
                macroquad::miniquad::window::clipboard_set(&format!(
                    "{} {link}",
                    t("Join my table in Snookered:")
                ));
                self.toast(t("Copied: send it to a friend."), true);
            }
            y += 58.0;
        }
        draw_line(area.x, y, area.x + area.w, y, 1.0, BORDER);
        y += 10.0;
        let others: Vec<TableInfo> =
            self.tables.iter().filter(|t| t.host.username != me).cloned().collect();
        let list = Rect::new(area.x, y, area.w, area.y + area.h - y);
        if others.is_empty() {
            ui.text(
                t("No open tables. Open one and others can sit down."),
                list.x,
                list.y + 8.0,
                14.0,
                MUTED,
                false,
            );
            return;
        }
        let row_h = 50.0;
        let visible = (list.h / row_h).floor().max(1.0) as usize;
        self.list_first = ui.scroll_list(list, self.list_first, others.len(), visible);
        let mut join = None;
        for (k, table) in others.iter().skip(self.list_first).take(visible).enumerate() {
            let r = Rect::new(list.x, list.y + k as f32 * row_h, list.w - 8.0, row_h - 6.0);
            rrect(r, 7.0, Color::new(1.0, 1.0, 1.0, 0.035));
            crate::icons::draw(Icon::Table, vec2(r.x + 20.0, r.y + r.h / 2.0), 22.0, CREAM);
            ui.player_name(&table.host, r.x + 40.0, r.y + 5.0, 15.0, r.w - 200.0);
            let mut px = r.x + 40.0;
            let (label, color) = if table.rated {
                (t("rated"), GOLD)
            } else {
                (t("casual"), Color::new(0.6, 0.8, 1.0, 1.0))
            };
            px += ui.pill(label, px, r.y + 24.0, color) + 6.0;
            ui.text(&table.host.rating.to_string(), px, r.y + 25.0, 12.0, MUTED, false);
            if table.private {
                crate::icons::draw(
                    Icon::Lock,
                    vec2(r.x + r.w - 118.0, r.y + r.h / 2.0),
                    16.0,
                    MUTED,
                );
            }
            let joining = self.joining.as_ref().is_some_and(|(id, _)| *id == table.id);
            if !joining
                && ui.button(
                    Rect::new(r.x + r.w - 96.0, r.y + 8.0, 88.0, r.h - 16.0),
                    t("Sit down"),
                    true,
                )
            {
                if table.private {
                    self.joining = Some((table.id, String::new()));
                } else {
                    join = Some((table.id, None));
                }
            }
            if joining {
                let ir = Rect::new(r.x + r.w - 250.0, r.y + 8.0, 150.0, r.h - 16.0);
                let (_, pw) = self.joining.as_mut().expect("joining");
                let enter = ui.password_input(32, ir, pw, 32);
                if pw.is_empty() && ui.focus != Some(32) {
                    ui.text(t("password"), ir.x + 8.0, ir.y + 9.0, 13.0, MUTED, false);
                }
                let pw = pw.clone();
                if ui.primary_button(
                    Rect::new(r.x + r.w - 96.0, r.y + 8.0, 88.0, r.h - 16.0),
                    t("Join"),
                ) || enter
                {
                    join = Some((table.id, Some(pw)));
                    self.joining = None;
                }
            }
        }
        if let Some((id, password)) = join {
            self.send(ClientMsg::JoinTable { id, password });
        }
    }

    fn draw_tournaments(&mut self, ui: &mut Ui, area: Rect) {
        if let Some(form) = &mut self.tournament_form {
            let mut y = area.y;
            ui.text(t("New tournament"), area.x, y, 17.0, GOLD, true);
            y += 32.0;
            ui.text(t("Name"), area.x, y + 9.0, 13.0, MUTED, true);
            ui.text_input(
                33,
                Rect::new(area.x + 90.0, y, area.w - 90.0, 32.0),
                &mut form.name,
                snookered_proto::MAX_NAME_LEN,
            );
            y += 44.0;
            ui.text(t("Format"), area.x, y + 8.0, 13.0, MUTED, true);
            let formats = [t("Knockout"), t("Arena")];
            let arena = form.format == snookered_proto::TournamentFormat::Arena;
            if let Some(i) =
                ui.segmented(Rect::new(area.x + 90.0, y, 220.0, 30.0), &formats, arena as usize)
            {
                form.format = if i == 1 {
                    snookered_proto::TournamentFormat::Arena
                } else {
                    snookered_proto::TournamentFormat::Knockout
                };
            }
            y += 42.0;
            if arena {
                for line in ui.wrap(t("30 minutes: play as many games as you can. A win is 2 points, 4 from the third win in a row."), area.w, 12.0) {
                    ui.text(&line, area.x, y, 12.0, MUTED, false);
                    y += 16.0;
                }
                y += 8.0;
            } else {
                ui.text(t("Players"), area.x, y + 8.0, 13.0, MUTED, true);
                let sizes = [4u32, 8, 16];
                let labels = ["4", "8", "16 (VIP)"];
                let sel = sizes.iter().position(|s| *s == form.size).unwrap_or(1);
                if let Some(i) =
                    ui.segmented(Rect::new(area.x + 90.0, y, 220.0, 30.0), &labels, sel)
                {
                    form.size = sizes[i];
                }
                y += 42.0;
            }
            if !self.lan {
                ui.text(t("Entry"), area.x, y + 8.0, 13.0, MUTED, true);
                let fees = [0u32, 50, 100, 250, 500];
                let labels: Vec<String> = fees
                    .iter()
                    .map(|f| if *f == 0 { t("Free").to_string() } else { f.to_string() })
                    .collect();
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                let sel = fees.iter().position(|f| *f == form.entry_fee).unwrap_or(0);
                if let Some(i) =
                    ui.segmented(Rect::new(area.x + 90.0, y, area.w - 90.0, 30.0), &labels, sel)
                {
                    form.entry_fee = fees[i];
                }
                y += 36.0;
                if form.entry_fee > 0 {
                    let note =
                        t("The fees make the prize: 70% to the champion, 20% to the runner-up.");
                    for line in ui.wrap(note, area.w, 12.0) {
                        ui.text(&line, area.x, y, 12.0, MUTED, false);
                        y += 16.0;
                    }
                }
                y += 8.0;
            }
            if !self.lan {
                ui.text(t("Games"), area.x, y + 8.0, 13.0, MUTED, true);
                let opts = [t("Rated"), t("Casual")];
                if let Some(i) = ui.segmented(
                    Rect::new(area.x + 90.0, y, 180.0, 30.0),
                    &opts,
                    if form.rated { 0 } else { 1 },
                ) {
                    form.rated = i == 0;
                }
                y += 42.0;
            }
            ui.text(t("Password"), area.x, y + 9.0, 13.0, MUTED, true);
            ui.text_input(
                34,
                Rect::new(area.x + 90.0, y, area.w - 90.0, 32.0),
                &mut form.password,
                32,
            );
            if form.password.is_empty() && ui.focus != Some(34) {
                ui.text(
                    t("optional: only friends with it can join"),
                    area.x + 98.0,
                    y + 9.0,
                    13.0,
                    MUTED,
                    false,
                );
            }
            y += 50.0;
            let ok = form.name.trim().chars().count() >= 3;
            let mut create = None;
            if ui.primary_button(Rect::new(area.x, y, area.w / 2.0 - 5.0, 38.0), t("Create")) && ok
            {
                create = Some(ClientMsg::CreateTournament {
                    name: form.name.clone(),
                    size: form.size,
                    rated: form.rated && !self.lan,
                    password: (!form.password.trim().is_empty()).then(|| form.password.clone()),
                    entry_fee: form.entry_fee,
                    format: form.format,
                });
            }
            let cancel = ui.button(
                Rect::new(area.x + area.w / 2.0 + 5.0, y, area.w / 2.0 - 5.0, 38.0),
                t("Cancel"),
                true,
            );
            y += 50.0;
            ui.text(
                t("The tournament starts when it is full (the host can start it earlier)."),
                area.x,
                y,
                12.0,
                MUTED,
                false,
            );
            if let Some(msg) = create {
                self.send(msg);
                self.tournament_form = None;
            } else if cancel {
                self.tournament_form = None;
            }
            return;
        }
        // Development aid (screenshots): open the first running tournament.
        if self.selected_tournament.is_none() && crate::dev::flag("SNOOKERED_OPEN_TOURNAMENT") {
            self.selected_tournament =
                self.tournaments.iter().find(|t| t.state == TournamentState::Running).map(|t| t.id);
        }
        if let Some(id) = self.selected_tournament {
            match self.tournaments.iter().find(|t| t.id == id).cloned() {
                Some(tour) => return self.draw_tournament(ui, area, &tour),
                None => self.selected_tournament = None,
            }
        }
        if ui.button(Rect::new(area.x, area.y, area.w, 36.0), t("Create a tournament"), true) {
            self.tournament_form =
                Some(TournamentForm { size: 8, rated: !self.lan, ..Default::default() });
        }
        let list = Rect::new(area.x, area.y + 48.0, area.w, area.h - 48.0);
        if self.tournaments.is_empty() {
            ui.text(
                t("No tournaments right now. Create one!"),
                list.x,
                list.y + 8.0,
                14.0,
                MUTED,
                false,
            );
            return;
        }
        let row_h = 54.0;
        let visible = (list.h / row_h).floor().max(1.0) as usize;
        let mut order: Vec<&TournamentInfo> = self.tournaments.iter().collect();
        order.sort_by_key(|t| {
            (t.state != TournamentState::Registering, t.state == TournamentState::Finished, t.id)
        });
        self.list_first = ui.scroll_list(list, self.list_first, order.len(), visible);
        let mut open = None;
        for (k, tour) in order.iter().skip(self.list_first).take(visible).enumerate() {
            let r = Rect::new(list.x, list.y + k as f32 * row_h, list.w - 8.0, row_h - 6.0);
            let hover = ui.hovering(r);
            rrect(r, 7.0, Color::new(1.0, 1.0, 1.0, if hover { 0.07 } else { 0.035 }));
            crate::icons::draw(Icon::Trophy, vec2(r.x + 20.0, r.y + r.h / 2.0), 22.0, GOLD);
            let name = ui.ellipsize(&tour.name, r.w - 170.0, 15.0);
            ui.text(&name, r.x + 40.0, r.y + 6.0, 15.0, CREAM, true);
            let sub = tw(
                "by {host} · {n}/{size} players",
                &[("host", &tour.host), ("n", &tour.players.len()), ("size", &tour.size)],
            );
            ui.text(&sub, r.x + 40.0, r.y + 27.0, 12.0, MUTED, false);
            let (label, color) = state_label(tour.state);
            let pw = ui.text_size(label, 11.0, true).x + 12.0;
            ui.pill(label, r.x + r.w - pw - 10.0, r.y + 8.0, color);
            let mut ix = r.x + r.w - 20.0;
            if tour.private {
                crate::icons::draw(Icon::Lock, vec2(ix, r.y + 34.0), 13.0, MUTED);
                ix -= 20.0;
            }
            if tour.rated {
                crate::icons::draw(Icon::Star, vec2(ix, r.y + 34.0), 13.0, GOLD);
            }
            if hover && ui.clicked(r) {
                open = Some(tour.id);
            }
        }
        if let Some(id) = open {
            self.selected_tournament = Some(id);
            self.tournament_password.clear();
        }
    }

    fn draw_tournament(&mut self, ui: &mut Ui, area: Rect, tour: &TournamentInfo) {
        let me = self.me.as_ref().map(|m| m.username.clone()).unwrap_or_default();
        if ui.icon_button(
            Rect::new(area.x, area.y, 30.0, 30.0),
            Icon::Back,
            t("All tournaments"),
            true,
        ) {
            self.selected_tournament = None;
            return;
        }
        let name = ui.ellipsize(&tour.name, area.w - 140.0, 18.0);
        ui.text(&name, area.x + 40.0, area.y + 4.0, 18.0, GOLD, true);
        let (label, color) = state_label(tour.state);
        let pw = ui.text_size(label, 11.0, true).x + 12.0;
        ui.pill(label, area.x + area.w - pw, area.y + 7.0, color);
        let mut y = area.y + 38.0;
        let kind = if tour.rated { t("rated games") } else { t("casual games") };
        let arena = tour.format == snookered_proto::TournamentFormat::Arena;
        let line = if arena {
            tw(
                "Arena · host {host} · {n} players · {kind}",
                &[("host", &tour.host), ("n", &tour.players.len()), ("kind", &kind)],
            )
        } else {
            tw(
                "Host {host} · {n}/{size} players · {kind}",
                &[
                    ("host", &tour.host),
                    ("n", &tour.players.len()),
                    ("size", &tour.size),
                    ("kind", &kind),
                ],
            )
        };
        ui.text(&line, area.x, y, 13.0, MUTED, false);
        y += 20.0;
        // Entry, prizes and times.
        let mut facts = Vec::new();
        if tour.entry_fee > 0 {
            facts.push(tw("Entry {n} coins", &[("n", &tour.entry_fee)]));
            facts.push(tw("Prizes {a} + {b}", &[("a", &tour.prizes[0]), ("b", &tour.prizes[1])]));
        }
        let now = crate::game::unix_now();
        if let Some(start) = tour.starts_at.filter(|_| tour.state == TournamentState::Registering) {
            let mins = ((start - now).max(0) + 59) / 60;
            facts.push(tw("starts in {n} min", &[("n", &mins)]));
        }
        if let Some(end) = tour.ends_at.filter(|_| tour.state == TournamentState::Running) {
            let left = (end - now).max(0);
            facts.push(tw(
                "{m}:{s} left",
                &[("m", &(left / 60)), ("s", &format!("{:02}", left % 60))],
            ));
        }
        if !facts.is_empty() {
            ui.text(&facts.join("  ·  "), area.x, y, 13.0, GOLD, true);
            y += 20.0;
        }
        y += 4.0;
        let joined = tour.players.iter().any(|p| p.username == me);
        let is_host = tour.host == me;
        if tour.state == TournamentState::Registering {
            let bw = (area.w - 10.0) / 2.0;
            if joined {
                if ui.button(
                    Rect::new(area.x, y, bw, 34.0),
                    if is_host { t("Cancel tournament") } else { t("Leave") },
                    true,
                ) {
                    self.send(ClientMsg::LeaveTournament { id: tour.id });
                }
                if is_host
                    && ui.primary_button(Rect::new(area.x + bw + 10.0, y, bw, 34.0), t("Start now"))
                    && tour.players.len() >= 2
                {
                    self.send(ClientMsg::StartTournament { id: tour.id });
                }
            } else {
                let full = tour.players.len() >= tour.size as usize;
                if tour.private {
                    ui.password_input(
                        35,
                        Rect::new(area.x, y, bw, 34.0),
                        &mut self.tournament_password,
                        32,
                    );
                    if self.tournament_password.is_empty() && ui.focus != Some(35) {
                        ui.text(t("password"), area.x + 8.0, y + 10.0, 13.0, MUTED, false);
                    }
                }
                let jr = if tour.private {
                    Rect::new(area.x + bw + 10.0, y, bw, 34.0)
                } else {
                    Rect::new(area.x, y, area.w, 34.0)
                };
                let label = match (full, tour.entry_fee) {
                    (true, _) => t("Join the waiting list").to_string(),
                    (false, 0) => t("Join").to_string(),
                    (false, fee) => tw("Join · {n} coins", &[("n", &fee)]),
                };
                if ui.primary_button(jr, &label) {
                    let password = tour.private.then(|| self.tournament_password.clone());
                    self.send(ClientMsg::JoinTournament { id: tour.id, password });
                }
            }
            y += 46.0;
        } else if tour.state == TournamentState::Running && arena {
            // An arena can be joined while it runs.
            if !joined {
                let label = match tour.entry_fee {
                    0 => t("Join").to_string(),
                    fee => tw("Join · {n} coins", &[("n", &fee)]),
                };
                if ui.primary_button(Rect::new(area.x, y, area.w, 34.0), &label) {
                    self.send(ClientMsg::JoinTournament { id: tour.id, password: None });
                }
            } else {
                ui.text(
                    t("You are paired as soon as you and an opponent are free."),
                    area.x,
                    y,
                    13.0,
                    GOLD,
                    false,
                );
                y += 20.0;
                if ui.button(Rect::new(area.x, y, 140.0, 28.0), t("Withdraw"), true) {
                    self.send(ClientMsg::LeaveTournament { id: tour.id });
                }
            }
            y += 44.0;
        } else if tour.state == TournamentState::Running && joined {
            let alive = tour.champion.is_none()
                && !tour.rounds.iter().flatten().any(|m| {
                    m.winner.is_some_and(|w| m.players[1 - w].as_deref() == Some(me.as_str()))
                });
            if alive {
                ui.text(
                    t("Your next match starts by itself when both players are free."),
                    area.x,
                    y,
                    13.0,
                    GOLD,
                    false,
                );
                y += 20.0;
                if ui.button(Rect::new(area.x, y, 140.0, 28.0), t("Withdraw"), true) {
                    self.send(ClientMsg::LeaveTournament { id: tour.id });
                }
                y += 40.0;
            }
        }
        if let Some(champion) = &tour.champion {
            let r = Rect::new(area.x, y, area.w, 40.0);
            rrect(r, 8.0, Color::new(0.91, 0.75, 0.38, 0.14));
            crate::icons::draw(Icon::Trophy, vec2(r.x + 22.0, r.y + 20.0), 22.0, GOLD);
            ui.text(
                &tw("Champion: {name}", &[("name", champion)]),
                r.x + 42.0,
                r.y + 11.0,
                16.0,
                GOLD,
                true,
            );
            y += 50.0;
        }
        let rest = Rect::new(area.x, y, area.w, area.y + area.h - y);
        if arena && !tour.standings.is_empty() {
            // Points, best first; a flame for a winning streak.
            for (k, s) in tour.standings.iter().enumerate() {
                let ry = rest.y + k as f32 * 26.0;
                if ry + 24.0 > rest.y + rest.h {
                    break;
                }
                let color = if s.username == me { GOLD } else { CREAM };
                ui.text(&format!("{}.", k + 1), rest.x, ry + 2.0, 13.0, MUTED, false);
                ui.text(&s.username, rest.x + 28.0, ry, 15.0, color, true);
                if s.streak >= 2 {
                    let nw = ui.text_size(&s.username, 15.0, true).x;
                    ui.text(
                        &format!("×{}", s.streak),
                        rest.x + 34.0 + nw,
                        ry + 2.0,
                        12.0,
                        ALERT,
                        true,
                    );
                }
                let pts = tw("{n} pts · {g} games", &[("n", &s.points), ("g", &s.games)]);
                let pw = ui.text_size(&pts, 13.0, false).x;
                ui.text(&pts, rest.x + rest.w - pw, ry + 2.0, 13.0, MUTED, false);
            }
        } else if tour.rounds.is_empty() {
            // Sign-up list.
            for (k, p) in tour.players.iter().enumerate() {
                let ry = rest.y + k as f32 * 26.0;
                if ry + 24.0 > rest.y + rest.h {
                    break;
                }
                ui.text(&format!("{}.", k + 1), rest.x, ry + 2.0, 13.0, MUTED, false);
                ui.player_name(p, rest.x + 28.0, ry, 15.0, rest.w - 100.0);
                ui.text(
                    &p.rating.to_string(),
                    rest.x + rest.w - 50.0,
                    ry + 2.0,
                    13.0,
                    MUTED,
                    false,
                );
            }
        } else {
            draw_bracket(ui, rest, tour, &me);
        }
    }

    fn draw_friends(&mut self, ui: &mut Ui, area: Rect) -> Option<OnlineAction> {
        let mut y = area.y;
        let input = Rect::new(area.x, y, area.w - 110.0, 32.0);
        let enter = ui.text_input(36, input, &mut self.friend_input, 16);
        if self.friend_input.is_empty() && ui.focus != Some(36) {
            ui.text(
                t("Add a friend by username"),
                input.x + 9.0,
                input.y + 9.0,
                14.0,
                MUTED,
                false,
            );
        }
        let ok = valid_username(self.friend_input.trim());
        if (ui.button(Rect::new(area.x + area.w - 100.0, y, 100.0, 32.0), t("Add"), ok) || enter)
            && ok
        {
            self.send(ClientMsg::FriendRequest { username: self.friend_input.trim().to_string() });
            self.friend_input.clear();
        }
        y += 44.0;
        let mut action = None;
        let mut answer = None;
        for from in self.friends.incoming.clone() {
            let r = Rect::new(area.x, y, area.w, 40.0);
            rrect(r, 7.0, Color::new(0.91, 0.75, 0.38, 0.12));
            ui.text(
                &tw("{name} wants to be friends", &[("name", &from)]),
                r.x + 12.0,
                r.y + 12.0,
                14.0,
                CREAM,
                false,
            );
            if ui.primary_button(Rect::new(r.x + r.w - 180.0, r.y + 6.0, 84.0, 28.0), t("Accept")) {
                answer = Some((from.clone(), true));
            }
            if ui.button(Rect::new(r.x + r.w - 90.0, r.y + 6.0, 84.0, 28.0), t("Decline"), true) {
                answer = Some((from.clone(), false));
            }
            y += 46.0;
        }
        if let Some((username, accept)) = answer {
            self.send(ClientMsg::FriendAnswer { username, accept });
        }
        let list = Rect::new(area.x, y, area.w, area.y + area.h - y);
        let total = self.friends.friends.len() + self.friends.outgoing.len();
        if total == 0 {
            ui.text(
                t("No friends yet. Add someone you played with!"),
                list.x,
                list.y + 8.0,
                14.0,
                MUTED,
                false,
            );
        }
        let row_h = 44.0;
        let visible = (list.h / row_h).floor().max(1.0) as usize;
        self.list_first = ui.scroll_list(list, self.list_first, total, visible);
        let mut invite = None;
        let mut remove = None;
        let mut watch = None;
        let friends = self.friends.friends.clone();
        let outgoing = self.friends.outgoing.clone();
        let rows = friends.iter().map(Some).chain(outgoing.iter().map(|_| None));
        for (k, (friend, i)) in rows.zip(0..).skip(self.list_first).take(visible).enumerate() {
            let r = Rect::new(list.x, list.y + k as f32 * row_h, list.w - 8.0, row_h - 6.0);
            rrect(r, 7.0, Color::new(1.0, 1.0, 1.0, 0.035));
            match friend {
                Some(f) => {
                    if f.online {
                        Self::status_dot(&f.player, r.x + 12.0, r.y + r.h / 2.0);
                    } else {
                        draw_circle(
                            r.x + 12.0,
                            r.y + r.h / 2.0,
                            4.5,
                            Color::new(0.4, 0.4, 0.4, 1.0),
                        );
                    }
                    let name_r = Rect::new(r.x + 24.0, r.y, r.w - 170.0, r.h);
                    ui.player_name(&f.player, name_r.x, r.y + 4.0, 15.0, name_r.w);
                    let sub = if f.online {
                        Self::status_text(&f.player)
                    } else {
                        last_seen(f.last_seen)
                    };
                    ui.text(&sub, name_r.x, r.y + 22.0, 12.0, MUTED, false);
                    if ui.clicked(name_r) {
                        action = Some(OnlineAction::Profile(f.player.username.clone()));
                    }
                    if f.online && f.player.status == PlayerStatus::Playing {
                        // Watch their game.
                        if ui.icon_button(
                            Rect::new(r.x + r.w - 136.0, r.y + 6.0, 88.0, r.h - 12.0),
                            Icon::Eye,
                            t("Watch the game"),
                            true,
                        ) {
                            watch = Some(f.player.username.clone());
                        }
                    } else {
                        let can = f.online
                            && f.player.status == PlayerStatus::Available
                            && self.outgoing.is_none();
                        if ui.button(
                            Rect::new(r.x + r.w - 136.0, r.y + 6.0, 88.0, r.h - 12.0),
                            t("Invite"),
                            can,
                        ) {
                            invite = Some(f.player.username.clone());
                        }
                    }
                    if ui.icon_button_ex(
                        Rect::new(r.x + r.w - 40.0, r.y + 6.0, 32.0, r.h - 12.0),
                        Icon::Close,
                        t("Remove friend"),
                        true,
                        true,
                    ) {
                        remove = Some(f.player.username.clone());
                    }
                }
                None => {
                    let name = &outgoing[i - friends.len()];
                    ui.text(name, r.x + 24.0, r.y + 5.0, 15.0, MUTED, true);
                    ui.text(t("request sent"), r.x + 24.0, r.y + 22.0, 12.0, MUTED, false);
                    if ui.icon_button_ex(
                        Rect::new(r.x + r.w - 40.0, r.y + 6.0, 32.0, r.h - 12.0),
                        Icon::Close,
                        t("Cancel request"),
                        true,
                        true,
                    ) {
                        remove = Some(name.clone());
                    }
                }
            }
        }
        if let Some(to) = invite {
            self.send(ClientMsg::Invite { to, rated: self.rated_games });
        }
        if let Some(username) = remove {
            self.send(ClientMsg::FriendRemove { username });
        }
        if let Some(name) = watch {
            self.spectate(&name);
        }
        action
    }
}

/// "seen 5 min ago", "seen yesterday"… (friends only).
pub fn last_seen(at: i64) -> String {
    if at <= 0 {
        return t("offline").to_string();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let s = (now - at).max(0);
    let n = |v: i64| v.to_string();
    match s {
        0..60 => t("seen just now").to_string(),
        60..3600 => tw("seen {n} min ago", &[("n", &n(s / 60))]),
        3600..86_400 => tw("seen {n} h ago", &[("n", &n(s / 3600))]),
        86_400..172_800 => t("seen yesterday").to_string(),
        172_800..2_592_000 => tw("seen {n} days ago", &[("n", &n(s / 86_400))]),
        _ => tw("seen {n} months ago", &[("n", &n(s / 2_592_000))]),
    }
}

fn state_label(state: TournamentState) -> (&'static str, Color) {
    match state {
        TournamentState::Registering => (t("open"), Color::new(0.4, 0.9, 0.5, 1.0)),
        TournamentState::Running => (t("playing"), Color::new(1.0, 0.7, 0.3, 1.0)),
        TournamentState::Finished => (t("finished"), MUTED),
    }
}

/// The bracket: one column per round, each match a small card.
fn draw_bracket(ui: &mut Ui, area: Rect, tour: &TournamentInfo, me: &str) {
    ui.block(area);
    let rounds = tour.rounds.len();
    if rounds == 0 {
        return;
    }
    let col_w = (area.w / rounds as f32).min(200.0);
    let card_h = 38.0f32;
    let profile = |name: &str| tour.players.iter().find(|p| p.username == name);
    for (r, round) in tour.rounds.iter().enumerate() {
        let x = area.x + r as f32 * col_w;
        let title = if r + 1 == rounds {
            t("Final").to_string()
        } else if r + 2 == rounds {
            t("Semi-finals").to_string()
        } else {
            tw("Round {n}", &[("n", &(r + 1))])
        };
        ui.text(&title, x + 4.0, area.y, 12.0, MUTED, true);
        // Compact: no more than ~2.4 cards' height per first-round match.
        let first = tour.rounds[0].len().max(1) as f32;
        let height = (area.h - 20.0).min(first * card_h * 2.4);
        let slot_h = height / round.len().max(1) as f32;
        for (i, m) in round.iter().enumerate() {
            let cy = area.y + 20.0 + slot_h * (i as f32 + 0.5);
            let card = Rect::new(x + 4.0, cy - card_h / 2.0, col_w - 14.0, card_h);
            let involved = m.players.iter().any(|p| p.as_deref() == Some(me));
            rrect(card, 5.0, Color::new(0.0, 0.0, 0.0, if involved { 0.45 } else { 0.3 }));
            rrect_lines(
                card,
                5.0,
                1.0,
                if m.live { Color::new(1.0, 0.7, 0.3, 0.9) } else { BORDER },
            );
            for seat in 0..2 {
                let y = card.y + 2.0 + seat as f32 * 18.0;
                let won = m.winner == Some(seat);
                let lost = m.winner.is_some() && !won;
                match &m.players[seat] {
                    Some(name) => {
                        let color = match profile(name) {
                            Some(p) if !lost => name_color(p),
                            _ if lost => Color::new(0.5, 0.46, 0.42, 1.0),
                            _ => CREAM,
                        };
                        let country = profile(name).map(|p| p.country.clone()).unwrap_or_default();
                        ui.name_with_flag(
                            name,
                            &country,
                            color,
                            card.x + 6.0,
                            y,
                            12.0,
                            card.w - 24.0,
                        );
                        if won {
                            draw_circle(card.x + card.w - 9.0, y + 8.0, 3.5, GOLD);
                        }
                    }
                    None => ui.text("—", card.x + 6.0, y, 12.0, MUTED, false),
                }
            }
            if m.live {
                let pulse = 0.5 + 0.5 * (get_time() as f32 * 4.0).sin();
                draw_circle(
                    card.x + card.w - 9.0,
                    card.y + card_h / 2.0,
                    3.0,
                    Color::new(1.0, 0.7, 0.3, pulse),
                );
            }
            // Connector to the next round.
            if r + 1 < rounds {
                let nx = x + col_w + 4.0;
                draw_line(
                    card.x + card.w,
                    cy,
                    nx - 2.0,
                    cy,
                    1.0,
                    Color::new(0.45, 0.31, 0.16, 0.6),
                );
            }
        }
    }
}

fn same_players(a: &Match, b: &Match) -> bool {
    let mut x = a.names.clone();
    let mut y = b.names.clone();
    x.sort();
    y.sort();
    x == y
}

/// Draws the round trip to the server, small, in a corner (during matches).
pub fn draw_ping(ui: &Ui, rtt: Option<std::time::Duration>, x_right: f32, y: f32) {
    let Some(rtt) = rtt else { return };
    let ms = rtt.as_millis();
    let color = if ms < 80 {
        Color::new(0.45, 0.9, 0.5, 0.85)
    } else if ms < 200 {
        Color::new(1.0, 0.8, 0.35, 0.9)
    } else {
        Color::new(1.0, 0.45, 0.4, 0.95)
    };
    let text = format!("{ms} ms");
    let w = ui.text_size(&text, 12.0, true).x;
    draw_circle(x_right - w - 10.0, y + 7.0, 3.5, color);
    ui.text(&text, x_right - w, y, 12.0, color, true);
}
