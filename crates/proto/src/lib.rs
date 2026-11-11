//! # snook_proto
//!
//! Messages exchanged between the Snook client and server over a
//! WebSocket, encoded as JSON (one message per text frame), plus the data
//! both sides must agree on: name rules, countries, rating tiers and the
//! achievement catalogue.
//!
//! Floats are serialized with exact round-tripping so game snapshots arrive
//! bit-identical on every machine.

mod achievements;
mod countries;

pub use achievements::{ACHIEVEMENTS, Achievement, Stat, Stats};
pub use countries::{COUNTRIES, FLAG_COLUMNS, FLAG_H, FLAG_W};

use glam::DVec2;
use serde::{Deserialize, Serialize};
use snook_core::{EightBallGame, GameOverReason, PocketId, Shot, ShotReport};

/// Bumped whenever messages change incompatibly.
pub const PROTOCOL_VERSION: u32 = 5;

/// Username rules: 3–16 characters, letters, digits and underscores.
pub fn valid_username(name: &str) -> bool {
    (3..=16).contains(&name.chars().count())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Password rules: 8–72 bytes (the hashing limit), any characters, not
/// made only of digits or of one repeated character.
pub fn valid_password(password: &str) -> bool {
    let first = password.chars().next();
    (8..=72).contains(&password.len())
        && !password.chars().all(|c| c.is_ascii_digit())
        && !password.chars().all(|c| Some(c) == first)
}

/// A country code from [`COUNTRIES`] (or the empty string: not set).
pub fn valid_country(code: &str) -> bool {
    code.is_empty() || country_index(code).is_some()
}

/// Position of `code` in [`COUNTRIES`] (and in the flag atlas).
pub fn country_index(code: &str) -> Option<usize> {
    COUNTRIES.iter().position(|(c, _, _)| c.eq_ignore_ascii_case(code))
}

/// Maximum length of a chat message, in characters.
pub const MAX_CHAT_LEN: usize = 200;
/// Maximum length of a table or tournament name, in characters.
pub const MAX_NAME_LEN: usize = 32;
/// Private tables and tournaments: password length in characters.
pub const MAX_ROOM_PASSWORD: usize = 32;

/// Removes control and invisible formatting characters (bidirectional
/// overrides, zero-width joiners…) and surrounding whitespace, collapses
/// runs of spaces and truncates to `max` characters.
pub fn clean_text(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for c in text.chars() {
        if c.is_control() || is_invisible(c) {
            continue;
        }
        let space = c.is_whitespace();
        if space && last_space {
            continue;
        }
        out.push(if space { ' ' } else { c });
        last_space = space;
        if out.chars().count() >= max {
            break;
        }
    }
    out.trim().to_string()
}

fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}'
        | '\u{FEFF}' | '\u{00AD}' | '\u{034F}' | '\u{180E}')
}

/// Cleans a chat message (see [`clean_text`]).
pub fn clean_chat(text: &str) -> String {
    clean_text(text, MAX_CHAT_LEN)
}

/// Rating tiers: shown as the colour of a player's name everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Rookie,
    Amateur,
    Skilled,
    Expert,
    Master,
    Grandmaster,
    Legend,
}

impl Tier {
    pub const ALL: [Tier; 7] = [
        Tier::Rookie,
        Tier::Amateur,
        Tier::Skilled,
        Tier::Expert,
        Tier::Master,
        Tier::Grandmaster,
        Tier::Legend,
    ];

    /// The tier for a rating.
    pub fn of(rating: i32) -> Tier {
        Tier::ALL.into_iter().rev().find(|t| rating >= t.min_rating()).unwrap_or(Tier::Rookie)
    }

    pub fn min_rating(self) -> i32 {
        match self {
            Tier::Rookie => i32::MIN,
            Tier::Amateur => 1100,
            Tier::Skilled => 1300,
            Tier::Expert => 1500,
            Tier::Master => 1700,
            Tier::Grandmaster => 1900,
            Tier::Legend => 2100,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tier::Rookie => "Rookie",
            Tier::Amateur => "Amateur",
            Tier::Skilled => "Skilled",
            Tier::Expert => "Expert",
            Tier::Master => "Master",
            Tier::Grandmaster => "Grandmaster",
            Tier::Legend => "Legend",
        }
    }
}

/// Account roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Player,
    Moderator,
    Admin,
}

impl Role {
    pub fn from_i16(v: i16) -> Role {
        match v {
            2 => Role::Admin,
            1 => Role::Moderator,
            _ => Role::Player,
        }
    }

    pub fn as_i16(self) -> i16 {
        self as i16
    }

    pub fn is_staff(self) -> bool {
        self >= Role::Moderator
    }
}

/// What a player is doing in the lounge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerStatus {
    Available,
    /// Seated at a table, waiting for an opponent.
    Waiting,
    Playing,
}

/// Public information about a player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub username: String,
    pub rating: i32,
    pub games: u32,
    pub wins: u32,
    pub status: PlayerStatus,
    /// ISO 3166-1 alpha-2 code, or empty.
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub role: Role,
    /// A guest on a LAN server (no account, no rating).
    #[serde(default)]
    pub guest: bool,
}

impl PlayerInfo {
    pub fn tier(&self) -> Tier {
        if self.guest { Tier::Rookie } else { Tier::of(self.rating) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub username: String,
    pub rating: i32,
    pub games: u32,
    pub wins: u32,
    #[serde(default)]
    pub country: String,
}

/// Who wrote a chat line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    #[default]
    Player,
    /// An announcement from the staff (shown highlighted).
    Announcement,
    /// Something that happened in the game (shown dimmed, without a name).
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
    /// Unix time in seconds.
    pub at: i64,
    #[serde(default)]
    pub kind: ChatKind,
    /// The writer's rating (for the name colour); 0 when unknown.
    #[serde(default)]
    pub rating: i32,
}

/// The aim of the player at the table, streamed so the opponent sees the cue move.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AimState {
    pub yaw: f64,
    pub elevation: f64,
    pub tip: DVec2,
    pub power: f32,
    /// The pocket called for the 8-ball, if any (used if time runs out).
    #[serde(default)]
    pub called_pocket: Option<PocketId>,
}

impl AimState {
    pub fn is_finite(&self) -> bool {
        self.yaw.is_finite()
            && self.elevation.is_finite()
            && self.tip.is_finite()
            && self.power.is_finite()
    }
}

/// A table in the lounge that someone opened and is waiting at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableInfo {
    pub id: u64,
    pub host: PlayerInfo,
    pub rated: bool,
    /// Joining needs a password.
    pub private: bool,
}

/// One match of a tournament bracket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BracketMatch {
    /// Usernames; `None` is a bye (or a player not decided yet).
    pub players: [Option<String>; 2],
    pub winner: Option<usize>,
    /// Being played right now.
    #[serde(default)]
    pub live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentState {
    Registering,
    Running,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TournamentInfo {
    pub id: u64,
    pub name: String,
    pub host: String,
    /// Maximum number of players (4, 8 or 16).
    pub size: u32,
    pub rated: bool,
    pub private: bool,
    pub state: TournamentState,
    pub players: Vec<PlayerInfo>,
    /// Rounds of the bracket, first round first (empty while registering).
    pub rounds: Vec<Vec<BracketMatch>>,
    pub champion: Option<String>,
}

/// A finished game in a player's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub opponent: String,
    pub opponent_country: String,
    pub opponent_rating: i32,
    pub won: bool,
    pub reason: GameOverReason,
    pub rated: bool,
    /// Rating change for this player (zero for casual games).
    pub delta: i32,
    /// This player's rating after the game.
    pub rating_after: i32,
    pub tournament: bool,
    /// Unix time in seconds.
    pub at: i64,
}

/// How the viewer relates to a profile's owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    #[default]
    None,
    /// The viewer's own profile.
    Me,
    Friend,
    /// The viewer asked to be friends.
    RequestSent,
    /// The owner asked the viewer to be friends.
    RequestReceived,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub player: PlayerInfo,
    pub online: bool,
    pub stats: Stats,
    /// Unix time in seconds.
    pub created_at: i64,
    /// Rank in the global leaderboard (None while provisional / no games).
    pub rank: Option<u32>,
    /// Unlocked achievement codes with the unlock time.
    pub achievements: Vec<(String, i64)>,
    pub history: Vec<HistoryEntry>,
    pub relation: Relation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriendInfo {
    pub player: PlayerInfo,
    pub online: bool,
}

/// What a player may report another for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportReason {
    Harassment,
    Cheating,
    OffensiveName,
    Spam,
    Other,
}

impl ReportReason {
    pub const ALL: [ReportReason; 5] = [
        ReportReason::Harassment,
        ReportReason::Cheating,
        ReportReason::OffensiveName,
        ReportReason::Spam,
        ReportReason::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ReportReason::Harassment => "Harassment",
            ReportReason::Cheating => "Cheating or abuse",
            ReportReason::OffensiveName => "Offensive name",
            ReportReason::Spam => "Spam",
            ReportReason::Other => "Other",
        }
    }
}

/// Messages from the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Must be the first message.
    Hello {
        version: u32,
    },
    Register {
        username: String,
        password: String,
        #[serde(default)]
        country: String,
    },
    Login {
        username: String,
        password: String,
    },
    /// Log in again with a token from an earlier `AuthOk`.
    Resume {
        token: String,
    },
    /// Join a LAN server without an account (only where the server allows it).
    Guest {
        name: String,
    },
    Logout,
    /// Lounge chat.
    Chat {
        text: String,
    },
    /// Chat with the opponent during a match.
    MatchChat {
        text: String,
    },
    Invite {
        to: String,
        rated: bool,
    },
    CancelInvite {
        to: String,
    },
    AnswerInvite {
        from: String,
        accept: bool,
    },
    /// Opens a table in the lounge and waits there for an opponent.
    OpenTable {
        rated: bool,
        /// Set for a private table (friends join with the password).
        #[serde(default)]
        password: Option<String>,
    },
    JoinTable {
        id: u64,
        #[serde(default)]
        password: Option<String>,
    },
    CloseTable,
    /// Moves the cue ball while it is in hand (shown to the opponent).
    PlaceCueBall {
        position: DVec2,
    },
    /// Current aim (send at most ~20 times per second).
    Aim {
        aim: AimState,
    },
    /// Takes the shot. `cue_ball` is the final position when the ball was in
    /// hand; `turn` is the game's `shots_taken()` the shot was aimed on.
    Shoot {
        turn: u32,
        shot: Shot,
        called_pocket: Option<PocketId>,
        cue_ball: Option<DVec2>,
    },
    Resign,
    /// Ask the same opponent for another game after a match ends.
    Rematch,
    /// Go back to the lounge after a match ends.
    LeaveMatch,
    Leaderboard,
    Profile {
        username: String,
    },
    SetCountry {
        country: String,
    },
    Friends,
    FriendRequest {
        username: String,
    },
    FriendAnswer {
        username: String,
        accept: bool,
    },
    FriendRemove {
        username: String,
    },
    /// Hides a player's chat and refuses their invitations.
    Block {
        username: String,
    },
    Unblock {
        username: String,
    },
    Report {
        username: String,
        reason: ReportReason,
        #[serde(default)]
        details: String,
    },
    Tournaments,
    CreateTournament {
        name: String,
        size: u32,
        rated: bool,
        #[serde(default)]
        password: Option<String>,
    },
    JoinTournament {
        id: u64,
        #[serde(default)]
        password: Option<String>,
    },
    LeaveTournament {
        id: u64,
    },
    /// The host starts the tournament early (with byes for the empty seats).
    StartTournament {
        id: u64,
    },
    Ping,
}

/// Why a match ended, from the server's point of view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    pub winner: usize,
    pub reason: GameOverReason,
    pub rated: bool,
    /// Rating change for seats 0 and 1 (zero for casual games).
    pub rating_delta: [i32; 2],
    pub new_ratings: [i32; 2],
}

/// Messages from the server.
// Game snapshots make some variants large; messages are short-lived, so boxing
// them would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        motd: String,
        /// The server runs on a local network (guests, no ratings).
        #[serde(default)]
        lan: bool,
    },
    AuthOk {
        player: PlayerInfo,
        token: String,
    },
    AuthError {
        reason: String,
    },
    /// Everyone in the lounge (sent after login).
    Lounge {
        players: Vec<PlayerInfo>,
    },
    /// A player came online or changed status/rating.
    PlayerUpdated {
        player: PlayerInfo,
    },
    PlayerLeft {
        username: String,
    },
    ChatHistory {
        lines: Vec<ChatLine>,
    },
    Chat {
        line: ChatLine,
    },
    Invited {
        from: String,
        rated: bool,
    },
    InviteSent {
        to: String,
        rated: bool,
    },
    InviteCancelled {
        from: String,
    },
    InviteDeclined {
        by: String,
    },
    /// An invitation could not be sent or accepted.
    InviteFailed {
        reason: String,
    },
    /// All open tables (after login).
    Tables {
        tables: Vec<TableInfo>,
    },
    TableOpened {
        table: TableInfo,
    },
    TableClosed {
        id: u64,
    },
    /// A match began (or the player reconnected to one).
    MatchStarted {
        match_id: u64,
        seat: usize,
        players: [PlayerInfo; 2],
        rated: bool,
        game: EightBallGame,
        shot_clock_secs: u32,
        /// Seconds left on the current shot clock.
        clock_left: f32,
        /// Part of this tournament.
        #[serde(default)]
        tournament: Option<u64>,
    },
    OpponentAim {
        aim: AimState,
    },
    OpponentCueBall {
        position: DVec2,
    },
    /// A shot was accepted: animate it from the current table (with the cue
    /// ball at `cue_ball` if it was in hand). Sent before the server has
    /// simulated it, so both players see it straight away.
    ShotTaken {
        seat: usize,
        turn: u32,
        shot: Shot,
        called_pocket: Option<PocketId>,
        cue_ball: Option<DVec2>,
        /// The shot clock ran out: the server played the shot at full power
        /// in the direction the player was aiming.
        #[serde(default)]
        timeout: bool,
    },
    /// The authoritative outcome of shot `turn`: adopt `end` once the
    /// animation is over.
    ShotResult {
        turn: u32,
        end: EightBallGame,
        report: ShotReport,
        /// Seconds left on the next shot clock (it starts after the animation).
        clock_left: f32,
    },
    /// A shot was refused: go back to this table.
    ShotRejected {
        turn: u32,
        reason: String,
        game: EightBallGame,
    },
    /// The shot clock expired for `seat`.
    TimeFoul {
        seat: usize,
        game: EightBallGame,
        clock_left: f32,
    },
    MatchChat {
        line: ChatLine,
    },
    /// The opponent lost (or regained) their connection.
    OpponentConnection {
        connected: bool,
        grace_secs: u32,
    },
    MatchEnded {
        result: MatchResult,
    },
    RematchOffered,
    /// The opponent left after the game; the match room is closed.
    OpponentLeft,
    Leaderboard {
        entries: Vec<LeaderboardEntry>,
    },
    Profile {
        profile: Box<Profile>,
    },
    Friends {
        friends: Vec<FriendInfo>,
        /// Usernames that asked to be friends.
        incoming: Vec<String>,
        /// Usernames this player asked.
        outgoing: Vec<String>,
        blocked: Vec<String>,
    },
    FriendRequestReceived {
        from: String,
    },
    AchievementUnlocked {
        code: String,
    },
    Tournaments {
        tournaments: Vec<TournamentInfo>,
    },
    TournamentUpdated {
        tournament: TournamentInfo,
    },
    TournamentRemoved {
        id: u64,
    },
    /// A message for the player (e.g. "your report was received").
    Notice {
        text: String,
    },
    /// Something the player did was rejected.
    Error {
        message: String,
    },
    /// The connection is about to be closed by the server (signed in
    /// elsewhere, banned, server restarting…). Don't reconnect automatically.
    Kicked {
        reason: String,
    },
    Pong,
}

impl ClientMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("client messages serialize")
    }
}

impl ServerMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("server messages serialize")
    }
}

pub fn parse_client(text: &str) -> Result<ClientMsg, serde_json::Error> {
    serde_json::from_str(text)
}

pub fn parse_server(text: &str) -> Result<ServerMsg, serde_json::Error> {
    serde_json::from_str(text)
}

/// The LAN discovery beacon: a LAN server broadcasts this (as UTF-8) on
/// [`LAN_DISCOVERY_PORT`] about once a second.
pub const LAN_DISCOVERY_PORT: u16 = 47878;
/// Default port of a LAN server.
pub const LAN_GAME_PORT: u16 = 47879;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanBeacon {
    /// Always "snook".
    pub game: String,
    pub version: u32,
    /// Name of the host.
    pub host: String,
    pub port: u16,
    pub players: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_passwords() {
        assert!(valid_username("Ana_99"));
        assert!(!valid_username("ab"));
        assert!(!valid_username("with space"));
        assert!(!valid_username("ação"));
        assert!(valid_password("secret12"));
        assert!(!valid_password("short1"));
        assert!(!valid_password("12345678"));
        assert!(!valid_password("aaaaaaaa"));
    }

    #[test]
    fn text_is_cleaned() {
        assert_eq!(clean_chat("  hi\u{7}there \n"), "hithere");
        assert_eq!(clean_chat("a\u{202E}b\u{200B}c   d"), "abc d");
        assert_eq!(clean_chat(&"x".repeat(500)).chars().count(), MAX_CHAT_LEN);
    }

    #[test]
    fn countries_and_tiers() {
        assert!(valid_country("BR"));
        assert!(valid_country("br"));
        assert!(valid_country(""));
        assert!(!valid_country("XX"));
        assert!(COUNTRIES.len() > 200);
        assert_eq!(Tier::of(1200), Tier::Amateur);
        assert_eq!(Tier::of(900), Tier::Rookie);
        assert_eq!(Tier::of(2500), Tier::Legend);
        assert_eq!(Tier::of(1500), Tier::Expert);
    }

    #[test]
    fn messages_round_trip() {
        let game = EightBallGame::new(3, 0);
        let p = |name: &str| PlayerInfo {
            username: name.into(),
            rating: 1200,
            games: 0,
            wins: 0,
            status: PlayerStatus::Playing,
            country: "BR".into(),
            role: Role::Player,
            guest: false,
        };
        let msg = ServerMsg::MatchStarted {
            match_id: 7,
            seat: 1,
            players: [p("a"), p("b")],
            rated: true,
            game,
            shot_clock_secs: 60,
            clock_left: 60.0,
            tournament: None,
        };
        assert_eq!(parse_server(&msg.to_json()).unwrap(), msg);
        let c = ClientMsg::Shoot {
            turn: 3,
            shot: Shot::new(0.123456789, 3.3),
            called_pocket: Some(PocketId::FootLeft),
            cue_ball: None,
        };
        assert_eq!(parse_client(&c.to_json()).unwrap(), c);
        assert!(c.to_json().contains("\"type\":\"shoot\""));
    }
}
