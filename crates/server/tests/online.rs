//! End-to-end tests: real WebSocket clients against a server on a random port.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use snook_core::{Difficulty, EightBallGame, ai};
use snook_proto::{ClientMsg, PROTOCOL_VERSION, ServerMsg, parse_server};
use snook_server::lounge::Rules;
use snook_server::{Config, Server, serve};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn start(rules: Rules) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = Config { bind: addr, database: None, rules };
    let server = Server::new(&config).unwrap();
    tokio::spawn(serve(listener, server));
    format!("ws://{addr}/ws")
}

struct Client {
    ws: Ws,
}

impl Client {
    async fn connect(url: &str) -> Client {
        let (ws, _) = connect_async(url).await.unwrap();
        let mut c = Client { ws };
        c.send(ClientMsg::Hello { version: PROTOCOL_VERSION }).await;
        c
    }

    async fn send(&mut self, msg: ClientMsg) {
        self.ws.send(Message::Text(msg.to_json().into())).await.unwrap();
    }

    /// Reads messages until one matches `pred`.
    async fn expect<F: Fn(&ServerMsg) -> bool>(&mut self, what: &str, pred: F) -> ServerMsg {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let msg = tokio::time::timeout(left, self.ws.next())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
                .expect("connection closed")
                .unwrap();
            if let Message::Text(t) = msg {
                let m = parse_server(t.as_str()).unwrap();
                if pred(&m) {
                    return m;
                }
            }
        }
    }

    async fn register(url: &str, name: &str) -> (Client, String) {
        let mut c = Client::connect(url).await;
        c.send(ClientMsg::Register { username: name.into(), password: "secret123".into() }).await;
        let ServerMsg::AuthOk { token, player } =
            c.expect("auth", |m| matches!(m, ServerMsg::AuthOk { .. })).await
        else {
            unreachable!()
        };
        assert_eq!(player.rating, 1200);
        (c, token)
    }
}

/// Invites `b` from `a`, accepts, and returns (game, seat of a, seat of b).
async fn start_match(
    a: &mut Client,
    b: &mut Client,
    b_name: &str,
    a_name: &str,
    rated: bool,
) -> (EightBallGame, usize, usize) {
    a.send(ClientMsg::Invite { to: b_name.into(), rated }).await;
    b.expect("invite", |m| matches!(m, ServerMsg::Invited { .. })).await;
    b.send(ClientMsg::AnswerInvite { from: a_name.into(), accept: true }).await;
    let ServerMsg::MatchStarted { game, seat: sa, .. } =
        a.expect("match a", |m| matches!(m, ServerMsg::MatchStarted { .. })).await
    else {
        unreachable!()
    };
    let ServerMsg::MatchStarted { seat: sb, .. } =
        b.expect("match b", |m| matches!(m, ServerMsg::MatchStarted { .. })).await
    else {
        unreachable!()
    };
    assert_ne!(sa, sb);
    (game, sa, sb)
}

#[tokio::test]
async fn full_rated_match_updates_ratings_and_leaderboard() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;

    // Lounge chat reaches everyone.
    alice.send(ClientMsg::Chat { text: "hello  ".into() }).await;
    let ServerMsg::Chat { line } =
        bob.expect("chat", |m| matches!(m, ServerMsg::Chat { .. })).await
    else {
        unreachable!()
    };
    assert_eq!((line.from.as_str(), line.text.as_str()), ("alice", "hello"));

    let (mut game, sa, _sb) = start_match(&mut alice, &mut bob, "bob", "alice", true).await;
    let mut shots = 0;
    let result = loop {
        assert!(shots < 400, "game did not finish");
        let shooter_is_alice = game.current_player() == sa;
        let decision = ai::decide(&game, Difficulty::Hard, shots);
        let msg = ClientMsg::Shoot {
            shot: decision.shot,
            called_pocket: decision.called_pocket,
            cue_ball: decision.cue_ball,
        };
        if shooter_is_alice {
            alice.send(msg).await;
        } else {
            bob.send(msg).await;
        }
        shots += 1;
        let played = alice
            .expect("shot", |m| matches!(m, ServerMsg::ShotPlayed { .. } | ServerMsg::Error { .. }))
            .await;
        let ServerMsg::ShotPlayed { end, .. } = played else { panic!("shot rejected: {played:?}") };
        let ServerMsg::ShotPlayed { end: end_b, .. } =
            bob.expect("shot b", |m| matches!(m, ServerMsg::ShotPlayed { .. })).await
        else {
            unreachable!()
        };
        assert_eq!(end, end_b, "both players get the same authoritative state");
        game = end;
        if game.game_over().is_some() {
            let ServerMsg::MatchEnded { result } =
                alice.expect("end", |m| matches!(m, ServerMsg::MatchEnded { .. })).await
            else {
                unreachable!()
            };
            break result;
        }
    };
    assert!(result.rated);
    assert!(result.rating_delta[result.winner] > 0);
    assert!(result.rating_delta[1 - result.winner] < 0);

    alice.send(ClientMsg::Leaderboard).await;
    let ServerMsg::Leaderboard { entries } =
        alice.expect("leaderboard", |m| matches!(m, ServerMsg::Leaderboard { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(entries.len(), 2);
    assert!(entries[0].rating > entries[1].rating);
    assert_eq!(entries[0].wins, 1);

    // Rematch needs both players.
    alice.send(ClientMsg::Rematch).await;
    bob.expect("rematch offer", |m| matches!(m, ServerMsg::RematchOffered)).await;
    bob.send(ClientMsg::Rematch).await;
    alice.expect("rematch", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
}

#[tokio::test]
async fn login_errors_and_validation() {
    let url = start(Rules::default()).await;
    let (_alice, token) = Client::register(&url, "Alice").await;

    let mut c = Client::connect(&url).await;
    c.send(ClientMsg::Register { username: "alice".into(), password: "secret123".into() }).await;
    let ServerMsg::AuthError { reason } =
        c.expect("taken", |m| matches!(m, ServerMsg::AuthError { .. })).await
    else {
        unreachable!()
    };
    assert!(reason.contains("taken"));
    c.send(ClientMsg::Register { username: "x".into(), password: "secret123".into() }).await;
    c.expect("bad name", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    c.send(ClientMsg::Login { username: "alice".into(), password: "nope!!".into() }).await;
    c.expect("bad password", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    c.send(ClientMsg::Login { username: "ALICE".into(), password: "secret123".into() }).await;
    c.expect("case-insensitive login", |m| matches!(m, ServerMsg::AuthOk { .. })).await;

    let mut d = Client::connect(&url).await;
    d.send(ClientMsg::Resume { token }).await;
    d.expect("token", |m| matches!(m, ServerMsg::AuthOk { .. })).await;

    let mut old = Client::connect(&url).await;
    old.send(ClientMsg::Hello { version: 0 }).await;
    old.expect("version", |m| matches!(m, ServerMsg::AuthError { .. })).await;
}

#[tokio::test]
async fn disconnect_reconnect_and_abandon() {
    let rules = Rules { reconnect_grace: Duration::from_secs(2), ..Rules::default() };
    let url = start(rules).await;
    let (mut alice, alice_token) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    let (_, _, _) = start_match(&mut alice, &mut bob, "bob", "alice", true).await;

    // Alice drops and comes back within the grace period.
    drop(alice);
    bob.expect("lost", |m| matches!(m, ServerMsg::OpponentConnection { connected: false, .. }))
        .await;
    let mut alice = Client::connect(&url).await;
    alice.send(ClientMsg::Resume { token: alice_token }).await;
    alice.expect("rejoined", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
    bob.expect("back", |m| matches!(m, ServerMsg::OpponentConnection { connected: true, .. }))
        .await;

    // Now she leaves for good: Bob wins.
    drop(alice);
    let ServerMsg::MatchEnded { result } =
        bob.expect("abandon", |m| matches!(m, ServerMsg::MatchEnded { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(result.reason, snook_core::GameOverReason::Abandoned);
}

#[tokio::test]
async fn shot_clock_plays_a_full_power_shot() {
    let rules = Rules { shot_clock: Duration::from_secs(1), ..Rules::default() };
    let url = start(rules).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    let (game, _, _) = start_match(&mut alice, &mut bob, "bob", "alice", false).await;
    let first = game.current_player();
    // Time runs out: the server plays the shot at full power.
    let ServerMsg::ShotPlayed { seat, shot, timeout, .. } =
        alice.expect("timeout shot", |m| matches!(m, ServerMsg::ShotPlayed { .. })).await
    else {
        unreachable!()
    };
    assert!(timeout);
    assert_eq!(seat, first);
    assert!((shot.speed - game.params().max_cue_speed).abs() < 1e-9);
}
