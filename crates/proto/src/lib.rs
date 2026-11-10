//! # snook_proto
//!
//! Messages exchanged between the Snook client and server over a
//! WebSocket, encoded as JSON (one message per text frame).
//!
//! Floats are serialized with exact round-tripping so game snapshots arrive
//! bit-identical on every machine.

use glam::DVec2;
use serde::{Deserialize, Serialize};
use snook_core::{EightBallGame, PocketId, Shot, ShotReport};

/// Bumped whenever messages change incompatibly.
pub const PROTOCOL_VERSION: u32 = 4;

/// Username rules: 3–16 characters, letters, digits and underscores.
pub fn valid_username(name: &str) -> bool {
    (3..=16).contains(&name.chars().count())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Password rules: 6–72 bytes (the hashing limit), any characters.
pub fn valid_password(password: &str) -> bool {
    (6..=72).contains(&password.len())
}

/// Maximum length of a chat message, in characters.
pub const MAX_CHAT_LEN: usize = 200;

/// Removes control characters and surrounding whitespace, and truncates.
pub fn clean_chat(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_CHAT_LEN)
        .collect::<String>()
        .trim()
        .to_string()
}

/// What a player is doing in the lounge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerStatus {
    Available,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub username: String,
    pub rating: i32,
    pub games: u32,
    pub wins: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
    /// Unix time in seconds.
    pub at: i64,
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
    },
    Login {
        username: String,
        password: String,
    },
    /// Log in again with a token from an earlier `AuthOk`.
    Resume {
        token: String,
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
    /// Moves the cue ball while it is in hand (shown to the opponent).
    PlaceCueBall {
        position: DVec2,
    },
    /// Current aim (send at most ~10 times per second).
    Aim {
        aim: AimState,
    },
    /// Takes the shot. `cue_ball` is the final position when the ball was in hand.
    Shoot {
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
    Ping,
}

/// Why a match ended, from the server's point of view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    pub winner: usize,
    pub reason: snook_core::GameOverReason,
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
    },
    AuthOk {
        player: PlayerInfo,
        token: String,
    },
    AuthError {
        reason: String,
    },
    /// Everyone in the lounge (sent after login and on request).
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
    },
    OpponentAim {
        aim: AimState,
    },
    OpponentCueBall {
        position: DVec2,
    },
    /// A shot was played. Clients animate from `start` and then adopt `end`.
    ShotPlayed {
        seat: usize,
        shot: Shot,
        called_pocket: Option<PocketId>,
        start: EightBallGame,
        end: EightBallGame,
        report: ShotReport,
        clock_left: f32,
        /// The shot clock ran out: the server played the shot at full power
        /// in the direction the player was aiming.
        #[serde(default)]
        timeout: bool,
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
    /// Something the player did was rejected.
    Error {
        message: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_passwords() {
        assert!(valid_username("Ana_99"));
        assert!(!valid_username("ab"));
        assert!(!valid_username("with space"));
        assert!(!valid_username("ação"));
        assert!(valid_password("secret1"));
        assert!(!valid_password("short"));
    }

    #[test]
    fn chat_is_cleaned() {
        assert_eq!(clean_chat("  hi\u{7}there \n"), "hithere");
        assert_eq!(clean_chat(&"x".repeat(500)).len(), MAX_CHAT_LEN);
    }

    #[test]
    fn messages_round_trip() {
        let game = EightBallGame::new(3, 0);
        let msg = ServerMsg::MatchStarted {
            match_id: 7,
            seat: 1,
            players: [
                PlayerInfo {
                    username: "a".into(),
                    rating: 1200,
                    games: 0,
                    wins: 0,
                    status: PlayerStatus::Playing,
                },
                PlayerInfo {
                    username: "b".into(),
                    rating: 1210,
                    games: 3,
                    wins: 2,
                    status: PlayerStatus::Playing,
                },
            ],
            rated: true,
            game,
            shot_clock_secs: 60,
            clock_left: 60.0,
        };
        assert_eq!(parse_server(&msg.to_json()).unwrap(), msg);
        let c = ClientMsg::Shoot {
            shot: Shot::new(0.123456789, 3.3),
            called_pocket: Some(PocketId::FootLeft),
            cue_ball: None,
        };
        assert_eq!(parse_client(&c.to_json()).unwrap(), c);
        assert!(c.to_json().contains("\"type\":\"shoot\""));
    }
}
