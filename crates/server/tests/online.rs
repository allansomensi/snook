//! End-to-end tests: real WebSocket clients against a server on a random port.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use snookered_core::{Difficulty, EightBallGame, ai};
use snookered_proto::{
    ClientMsg, PROTOCOL_VERSION, ReportReason, ServerMsg, TournamentState, parse_server,
};
use snookered_server::lounge::Rules;
use snookered_server::{Config, Server, serve};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const PASSWORD: &str = "secret123";

async fn start_with(rules: Rules, admins: Vec<String>) -> (String, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let rules = Rules { max_connections_per_ip: 100, ..rules };
    let config = Config { bind: addr, rules, admins, ..Config::default() };
    let server = Server::new(config).await.unwrap();
    tokio::spawn(serve(listener, server));
    (format!("ws://{addr}/ws"), format!("http://{addr}"))
}

async fn start(rules: Rules) -> String {
    start_with(rules, Vec::new()).await.0
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
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
        c.send(ClientMsg::Register {
            username: name.into(),
            password: PASSWORD.into(),
            country: "BR".into(),
        })
        .await;
        let ServerMsg::AuthOk { token, player } =
            c.expect("auth", |m| matches!(m, ServerMsg::AuthOk { .. })).await
        else {
            unreachable!()
        };
        assert_eq!(player.rating, 1200);
        assert_eq!(player.country, "BR");
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

/// Plays a whole game with the AI on both sides; returns the result.
async fn play_out(
    a: &mut Client,
    b: &mut Client,
    mut game: EightBallGame,
    sa: usize,
) -> snookered_proto::MatchResult {
    let mut shots = 0;
    loop {
        assert!(shots < 400, "game did not finish");
        let shooter_is_a = game.current_player() == sa;
        let decision = ai::decide(&game, Difficulty::Hard, shots);
        let turn = game.shots_taken();
        let msg = ClientMsg::Shoot {
            turn,
            shot: decision.shot,
            called_pocket: decision.called_pocket,
            cue_ball: decision.cue_ball,
        };
        if shooter_is_a {
            a.send(msg).await;
        } else {
            b.send(msg).await;
        }
        shots += 1;
        // Both players hear about the shot first, then get the outcome.
        for c in [&mut *a, &mut *b] {
            let taken = c
                .expect("shot taken", |m| {
                    matches!(m, ServerMsg::ShotTaken { .. } | ServerMsg::ShotRejected { .. })
                })
                .await;
            assert!(
                matches!(taken, ServerMsg::ShotTaken { turn: t, .. } if t == turn),
                "shot rejected: {taken:?}"
            );
        }
        let ServerMsg::ShotResult { end, turn: t, .. } =
            a.expect("result", |m| matches!(m, ServerMsg::ShotResult { .. })).await
        else {
            unreachable!()
        };
        assert_eq!(t, turn);
        let ServerMsg::ShotResult { end: end_b, .. } =
            b.expect("result b", |m| matches!(m, ServerMsg::ShotResult { .. })).await
        else {
            unreachable!()
        };
        assert_eq!(end, end_b, "both players get the same authoritative state");
        // The client simulation agrees with the server's.
        let mut local = game.clone();
        if let Some(p) = decision.cue_ball {
            local.place_cue_ball(p);
        }
        local.play_shot(&decision.shot, decision.called_pocket).unwrap();
        assert_eq!(local, end, "deterministic physics");
        game = end;
        if game.game_over().is_some() {
            let ServerMsg::MatchEnded { result } =
                a.expect("end", |m| matches!(m, ServerMsg::MatchEnded { .. })).await
            else {
                unreachable!()
            };
            b.expect("end b", |m| matches!(m, ServerMsg::MatchEnded { .. })).await;
            return result;
        }
    }
}

#[tokio::test]
async fn full_rated_match_updates_ratings_profile_and_leaderboard() {
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

    let (game, sa, _sb) = start_match(&mut alice, &mut bob, "bob", "alice", true).await;
    let result = play_out(&mut alice, &mut bob, game, sa).await;
    assert!(result.rated);
    assert!(result.rating_delta[result.winner] > 0);
    assert!(result.rating_delta[1 - result.winner] < 0);
    // The winner unlocks "first win".
    let winner_is_alice = result.winner == sa;
    let w = if winner_is_alice { &mut alice } else { &mut bob };
    w.expect(
        "achievement",
        |m| matches!(m, ServerMsg::AchievementUnlocked { code } if code == "wins_1"),
    )
    .await;

    alice.send(ClientMsg::Leaderboard).await;
    let ServerMsg::Leaderboard { entries } =
        alice.expect("leaderboard", |m| matches!(m, ServerMsg::Leaderboard { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(entries.len(), 2);
    assert!(entries[0].rating > entries[1].rating);
    assert_eq!(entries[0].wins, 1);
    assert_eq!(entries[0].country, "BR");

    alice.send(ClientMsg::Profile { username: "BOB".into() }).await;
    let ServerMsg::Profile { profile } =
        alice.expect("profile", |m| matches!(m, ServerMsg::Profile { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(profile.player.username, "bob");
    assert_eq!(profile.history.len(), 1);
    assert_eq!(profile.history[0].opponent, "alice");
    assert_eq!(profile.history[0].won, !winner_is_alice);
    assert!(profile.achievements.iter().any(|(c, _)| c == "games_1"));

    // Rematch needs both players.
    alice.send(ClientMsg::Rematch).await;
    bob.expect("rematch offer", |m| matches!(m, ServerMsg::RematchOffered)).await;
    bob.send(ClientMsg::Rematch).await;
    alice.expect("rematch", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
}

#[tokio::test]
async fn out_of_turn_and_stale_shots_are_rejected() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    let (game, sa, _) = start_match(&mut alice, &mut bob, "bob", "alice", false).await;
    let (shooter, other) =
        if game.current_player() == sa { (&mut alice, &mut bob) } else { (&mut bob, &mut alice) };
    let d = ai::decide(&game, Difficulty::Easy, 1);
    let shoot = |turn| ClientMsg::Shoot {
        turn,
        shot: d.shot,
        called_pocket: d.called_pocket,
        cue_ball: d.cue_ball,
    };
    other.send(shoot(0)).await;
    other.expect("not your turn", |m| matches!(m, ServerMsg::ShotRejected { .. })).await;
    shooter.send(shoot(5)).await;
    shooter.expect("stale", |m| matches!(m, ServerMsg::ShotRejected { .. })).await;
    // Infinity can't be written as JSON: send it by hand.
    let text = shoot(0).to_json().replace(&format!("{}", d.shot.yaw), "1e999");
    shooter.ws.send(Message::Text(text.into())).await.unwrap();
    let m = shooter
        .expect("bad", |m| {
            matches!(
                m,
                ServerMsg::ShotRejected { .. }
                    | ServerMsg::Error { .. }
                    | ServerMsg::ShotTaken { .. }
            )
        })
        .await;
    assert!(!matches!(m, ServerMsg::ShotTaken { .. }), "infinite shot accepted");
}

#[tokio::test]
async fn reactions_need_to_be_unlocked() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    alice.expect("achievements", |m| matches!(m, ServerMsg::MyAchievements { .. })).await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    start_match(&mut alice, &mut bob, "bob", "alice", false).await;
    // The crown needs a 1900 rating: dropped. The clap is for everyone.
    alice.send(ClientMsg::React { code: "crown".into() }).await;
    alice.send(ClientMsg::React { code: "nonsense".into() }).await;
    alice.send(ClientMsg::React { code: "clap".into() }).await;
    let ServerMsg::Reaction { code, .. } =
        bob.expect("reaction", |m| matches!(m, ServerMsg::Reaction { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(code, "clap");
    // A few a second at most.
    for _ in 0..10 {
        alice.send(ClientMsg::React { code: "wow".into() }).await;
    }
    alice.send(ClientMsg::MatchChat { text: "end".into() }).await;
    let mut seen = 0;
    while let ServerMsg::Reaction { .. } = bob
        .expect("more", |m| matches!(m, ServerMsg::Reaction { .. } | ServerMsg::MatchChat { .. }))
        .await
    {
        seen += 1;
    }
    assert!(seen <= 3, "{seen} reactions got through");
}

#[tokio::test]
async fn friends_can_watch_and_react() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    let (mut carol, _) = Client::register(&url, "carol").await;
    let (mut dave, _) = Client::register(&url, "dave").await;
    carol.send(ClientMsg::FriendRequest { username: "alice".into() }).await;
    alice.expect("request", |m| matches!(m, ServerMsg::FriendRequestReceived { .. })).await;
    alice.send(ClientMsg::FriendAnswer { username: "carol".into(), accept: true }).await;
    carol
        .expect(
            "friends",
            |m| matches!(m, ServerMsg::Friends { friends, .. } if !friends.is_empty()),
        )
        .await;
    let (game, sa, _) = start_match(&mut alice, &mut bob, "bob", "alice", false).await;

    // Not a friend: refused.
    dave.send(ClientMsg::Spectate { username: "alice".into() }).await;
    dave.expect("refused", |m| matches!(m, ServerMsg::Error { .. })).await;

    // A friend watches, from Alice's side; both players see the count.
    carol.send(ClientMsg::Spectate { username: "alice".into() }).await;
    let ServerMsg::MatchStarted { seat, spectating, .. } =
        carol.expect("watching", |m| matches!(m, ServerMsg::MatchStarted { .. })).await
    else {
        unreachable!()
    };
    assert!(spectating);
    assert_eq!(seat, sa);
    for c in [&mut alice, &mut bob] {
        c.expect("count", |m| matches!(m, ServerMsg::Spectators { count: 1 })).await;
    }

    // The spectator sees the shots and can react (quietly).
    let d = ai::decide(&game, Difficulty::Easy, 1);
    let shooter = if game.current_player() == sa { &mut alice } else { &mut bob };
    shooter
        .send(ClientMsg::Shoot {
            turn: 0,
            shot: d.shot,
            called_pocket: d.called_pocket,
            cue_ball: d.cue_ball,
        })
        .await;
    carol.expect("shot", |m| matches!(m, ServerMsg::ShotTaken { .. })).await;
    carol.expect("result", |m| matches!(m, ServerMsg::ShotResult { .. })).await;
    carol.send(ClientMsg::React { code: "clap".into() }).await;
    bob.expect(
        "crowd",
        |m| matches!(m, ServerMsg::Reaction { seat, .. } if *seat == snookered_proto::SPECTATOR_SEAT),
    )
    .await;
    // Carol cheers for Bob's seat: both players see the crowd's side.
    let bob_seat = 1 - sa;
    carol.send(ClientMsg::Cheer { seat: bob_seat }).await;
    alice
        .expect("support", |m| {
            matches!(m, ServerMsg::Support { counts } if counts[bob_seat] == 1 && counts[1 - bob_seat] == 0)
        })
        .await;
    // Players' chat stays between them.
    alice.send(ClientMsg::MatchChat { text: "psst".into() }).await;
    bob.expect("chat", |m| matches!(m, ServerMsg::MatchChat { .. })).await;

    // The game ends; when Alice goes back to the lounge, so does Carol.
    alice.send(ClientMsg::Resign).await;
    carol.expect("ended", |m| matches!(m, ServerMsg::MatchEnded { .. })).await;
    alice.send(ClientMsg::LeaveMatch).await;
    let m = carol
        .expect("back", |m| {
            matches!(m, ServerMsg::SpectateEnded { .. } | ServerMsg::MatchChat { .. })
        })
        .await;
    assert!(matches!(m, ServerMsg::SpectateEnded { .. }), "the spectator got the players' chat");
}

#[tokio::test]
async fn play_now_pairs_players_of_the_same_mode() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    let (mut carol, _) = Client::register(&url, "carol").await;
    // Carol wants a casual game: she isn't paired with the rated queue.
    carol.send(ClientMsg::QueueJoin { rated: false }).await;
    carol.expect("searching", |m| matches!(m, ServerMsg::Queue { searching: true, .. })).await;
    alice.send(ClientMsg::QueueJoin { rated: true }).await;
    alice.expect("searching", |m| matches!(m, ServerMsg::Queue { searching: true, .. })).await;
    bob.send(ClientMsg::QueueJoin { rated: true }).await;
    for c in [&mut alice, &mut bob] {
        let ServerMsg::MatchStarted { rated, players, .. } =
            c.expect("paired", |m| matches!(m, ServerMsg::MatchStarted { .. })).await
        else {
            unreachable!()
        };
        assert!(rated);
        let mut names: Vec<_> = players.iter().map(|p| p.username.clone()).collect();
        names.sort();
        assert_eq!(names, ["alice", "bob"]);
    }
    // Carol leaves the queue.
    carol.send(ClientMsg::QueueLeave).await;
    carol.expect("left", |m| matches!(m, ServerMsg::Queue { searching: false, .. })).await;
}

#[tokio::test]
async fn login_errors_and_validation() {
    let url = start(Rules::default()).await;
    let (_alice, token) = Client::register(&url, "Alice").await;

    // The previous version still plays; anything older is told to update.
    for (version, welcome) in [(snookered_proto::OLDEST_PROTOCOL_VERSION, true), (10, false)] {
        let (ws, _) = connect_async(&url).await.unwrap();
        let mut old = Client { ws };
        old.send(ClientMsg::Hello { version }).await;
        old.send(ClientMsg::Resume { token: token.clone() }).await;
        let m = old
            .expect("answer", |m| {
                matches!(m, ServerMsg::AuthOk { .. } | ServerMsg::AuthError { .. })
            })
            .await;
        assert_eq!(matches!(m, ServerMsg::AuthOk { .. }), welcome, "version {version}: {m:?}");
    }

    let mut c = Client::connect(&url).await;
    c.send(ClientMsg::Register {
        username: "alice".into(),
        password: PASSWORD.into(),
        country: String::new(),
    })
    .await;
    let ServerMsg::AuthError { reason } =
        c.expect("taken", |m| matches!(m, ServerMsg::AuthError { .. })).await
    else {
        unreachable!()
    };
    assert!(reason.contains("taken"));
    c.send(ClientMsg::Register {
        username: "admin".into(),
        password: PASSWORD.into(),
        country: String::new(),
    })
    .await;
    c.expect("reserved", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    c.send(ClientMsg::Login { username: "alice".into(), password: "nope!!nope".into() }).await;
    c.expect("bad password", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    c.send(ClientMsg::Login { username: "ALICE".into(), password: PASSWORD.into() }).await;
    c.expect("case-insensitive login", |m| matches!(m, ServerMsg::AuthOk { .. })).await;

    let mut d = Client::connect(&url).await;
    d.send(ClientMsg::Resume { token }).await;
    d.expect("token", |m| matches!(m, ServerMsg::AuthOk { .. })).await;
    // The older connection was replaced.
    c.expect("kicked", |m| matches!(m, ServerMsg::Kicked { .. })).await;

    let mut old = Client::connect(&url).await;
    old.send(ClientMsg::Hello { version: 0 }).await;
    old.expect("version", |m| matches!(m, ServerMsg::AuthError { .. })).await;

    // Password guessing is throttled per account.
    let mut e = Client::connect(&url).await;
    for _ in 0..4 {
        e.send(ClientMsg::Login { username: "alice".into(), password: "wrongpass1".into() }).await;
        e.expect("wrong", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    }
    let mut f = Client::connect(&url).await;
    for _ in 0..4 {
        f.send(ClientMsg::Login { username: "alice".into(), password: "wrongpass1".into() }).await;
        f.expect("wrong", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    }
    let mut g = Client::connect(&url).await;
    g.send(ClientMsg::Login { username: "alice".into(), password: PASSWORD.into() }).await;
    let ServerMsg::AuthError { reason } =
        g.expect("throttled", |m| matches!(m, ServerMsg::AuthError { .. })).await
    else {
        unreachable!()
    };
    assert!(reason.contains("Too many"), "{reason}");
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
    assert_eq!(result.reason, snookered_core::GameOverReason::Abandoned);
}

#[tokio::test]
async fn shot_clock_plays_a_full_power_shot() {
    let rules = Rules { shot_clock: Duration::from_secs(1), ..Rules::default() };
    let url = start(rules).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    let (game, _, _) = start_match(&mut alice, &mut bob, "bob", "alice", false).await;
    let first = game.current_player();
    let ServerMsg::ShotTaken { seat, shot, timeout, .. } =
        alice.expect("timeout shot", |m| matches!(m, ServerMsg::ShotTaken { .. })).await
    else {
        unreachable!()
    };
    assert!(timeout);
    assert_eq!(seat, first);
    assert!((shot.speed - game.params().max_cue_speed).abs() < 1e-9);
    alice.expect("result", |m| matches!(m, ServerMsg::ShotResult { .. })).await;
}

#[tokio::test]
async fn private_tables_need_the_password() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    alice.send(ClientMsg::OpenTable { rated: false, password: Some("pool night".into()) }).await;
    let ServerMsg::TableOpened { table } =
        bob.expect("table", |m| matches!(m, ServerMsg::TableOpened { .. })).await
    else {
        unreachable!()
    };
    assert!(table.private);
    bob.send(ClientMsg::JoinTable { id: table.id, password: Some("wrong".into()) }).await;
    bob.expect("wrong password", |m| matches!(m, ServerMsg::Error { .. })).await;
    bob.send(ClientMsg::JoinTable { id: table.id, password: Some("pool night".into()) }).await;
    bob.expect("started", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
    alice.expect("started", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
}

#[tokio::test]
async fn friends_blocks_and_reports() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    alice.send(ClientMsg::FriendRequest { username: "bob".into() }).await;
    bob.expect("request", |m| matches!(m, ServerMsg::FriendRequestReceived { .. })).await;
    bob.send(ClientMsg::FriendAnswer { username: "alice".into(), accept: true }).await;
    alice
        .expect(
            "friend achievement",
            |m| matches!(m, ServerMsg::AchievementUnlocked { code } if code == "friend_1"),
        )
        .await;
    let ServerMsg::Friends { friends, .. } = alice
        .expect(
            "friends",
            |m| matches!(m, ServerMsg::Friends { friends, .. } if !friends.is_empty()),
        )
        .await
    else {
        unreachable!()
    };
    assert_eq!(friends[0].player.username, "bob");
    assert!(friends[0].online);

    // Bob blocks Alice: her invitations are refused and the friendship ends.
    bob.send(ClientMsg::Block { username: "alice".into() }).await;
    bob.expect(
        "blocked",
        |m| matches!(m, ServerMsg::Friends { blocked, .. } if !blocked.is_empty()),
    )
    .await;
    alice.send(ClientMsg::Invite { to: "bob".into(), rated: false }).await;
    alice.expect("refused", |m| matches!(m, ServerMsg::InviteFailed { .. })).await;
    alice
        .send(ClientMsg::Report {
            username: "bob".into(),
            reason: ReportReason::Spam,
            details: "test".into(),
        })
        .await;
    alice.expect("report", |m| matches!(m, ServerMsg::Notice { .. })).await;
}

#[tokio::test]
async fn tournament_runs_to_a_champion() {
    // The two semi-finals are played one after the other: a long shot clock
    // keeps the waiting one from playing time-out shots meanwhile (slow CI
    // machines take more than 30 s for a game).
    let rules = Rules { shot_clock: Duration::from_secs(600), ..Rules::default() };
    let url = start(rules).await;
    let mut clients = Vec::new();
    for name in ["ana", "ben", "cid", "dan"] {
        clients.push((name, Client::register(&url, name).await.0));
    }
    clients[0]
        .1
        .send(ClientMsg::CreateTournament {
            name: "Friday cup".into(),
            size: 4,
            rated: false,
            password: None,
            entry_fee: 0,
            format: Default::default(),
        })
        .await;
    let ServerMsg::TournamentUpdated { tournament } =
        clients[1].1.expect("created", |m| matches!(m, ServerMsg::TournamentUpdated { .. })).await
    else {
        unreachable!()
    };
    for (_, c) in clients.iter_mut().skip(1) {
        c.send(ClientMsg::JoinTournament { id: tournament.id, password: None }).await;
    }
    // Full: it starts by itself and two semi-finals begin.
    let mut champion = None;
    for _round in 0..2 {
        let mut games = Vec::new();
        for (i, (_, c)) in clients.iter_mut().enumerate() {
            let started = tokio::time::timeout(
                Duration::from_secs(3),
                c.expect("match", |m| matches!(m, ServerMsg::MatchStarted { .. })),
            )
            .await;
            if let Ok(ServerMsg::MatchStarted { game, seat, players, .. }) = started {
                games.push((i, game, seat, players));
            }
        }
        // Pair up the players of each match and play it out.
        while let Some((i, game, seat, players)) = games.pop() {
            let opp = players[1 - seat].username.clone();
            let j_pos = games.iter().position(|(j, ..)| clients[*j].0 == opp).expect("opponent");
            let (j, ..) = games.remove(j_pos);
            let (lo, hi) = (i.min(j), i.max(j));
            let (left, right) = clients.split_at_mut(hi);
            let (a, b) = (&mut left[lo].1, &mut right[0].1);
            let a_seat = if lo == i { seat } else { 1 - seat };
            let result = play_out(a, b, game, a_seat).await;
            a.send(ClientMsg::LeaveMatch).await;
            b.send(ClientMsg::LeaveMatch).await;
            let _ = result;
        }
        let c = &mut clients[0].1;
        let t = c
            .expect("bracket", |m| {
                matches!(m, ServerMsg::TournamentUpdated { tournament }
                    if tournament.state == TournamentState::Finished
                        || tournament.rounds.get(1).is_some_and(|r| r[0].players.iter().all(Option::is_some)))
            })
            .await;
        if let ServerMsg::TournamentUpdated { tournament } = t
            && tournament.state == TournamentState::Finished
        {
            champion = tournament.champion;
            break;
        }
    }
    if champion.is_none() {
        let ServerMsg::TournamentUpdated { tournament } = clients[0]
            .1
            .expect("finished", |m| {
                matches!(m, ServerMsg::TournamentUpdated { tournament } if tournament.state == TournamentState::Finished)
            })
            .await
        else {
            unreachable!()
        };
        champion = tournament.champion;
    }
    assert!(champion.is_some());
}

#[tokio::test]
async fn lan_guests_play_casual_games() {
    let rules = Rules { lan: true, ..Rules::default() };
    let url = start(rules).await;
    let mut a = Client::connect(&url).await;
    a.send(ClientMsg::Register {
        username: "x_y".into(),
        password: PASSWORD.into(),
        country: String::new(),
    })
    .await;
    a.expect("no accounts", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    a.send(ClientMsg::Guest { name: "host".into() }).await;
    let ServerMsg::AuthOk { player, .. } =
        a.expect("guest", |m| matches!(m, ServerMsg::AuthOk { .. })).await
    else {
        unreachable!()
    };
    assert!(player.guest);
    let mut b = Client::connect(&url).await;
    b.send(ClientMsg::Guest { name: "HOST".into() }).await;
    b.expect("name taken", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    b.send(ClientMsg::Guest { name: "visitor".into() }).await;
    b.expect("guest", |m| matches!(m, ServerMsg::AuthOk { .. })).await;
    a.send(ClientMsg::Invite { to: "visitor".into(), rated: true }).await;
    let ServerMsg::Invited { rated, .. } =
        b.expect("invite", |m| matches!(m, ServerMsg::Invited { .. })).await
    else {
        unreachable!()
    };
    assert!(!rated, "LAN games are never rated");
}

#[tokio::test]
async fn admin_panel_requires_staff() {
    let (url, http) = start_with(Rules::default(), vec!["boss".into()]).await;
    let (mut boss, _) = Client::register(&url, "boss").await;
    let (_eve, _) = Client::register(&url, "eve").await;
    let host = http.trim_start_matches("http://").to_string();
    // A minimal HTTP client over a raw socket (no extra dependencies).
    async fn request(
        host: &str,
        method: &str,
        path: &str,
        token: &str,
        body: &str,
    ) -> (u16, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = TcpStream::connect(host).await.unwrap();
        let auth = if token.is_empty() {
            String::new()
        } else {
            format!("Authorization: Bearer {token}\r\n")
        };
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        let code = out[9..12].parse().unwrap();
        let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (code, body)
    }
    let (code, _) = request(&host, "GET", "/admin/api/overview", "", "").await;
    assert_eq!(code, 401);
    let (code, _) = request(
        &host,
        "POST",
        "/admin/api/login",
        "",
        &format!(r#"{{"username":"eve","password":"{PASSWORD}"}}"#),
    )
    .await;
    assert_eq!(code, 403, "players can't sign in to the panel");
    let (code, body) = request(
        &host,
        "POST",
        "/admin/api/login",
        "",
        &format!(r#"{{"username":"boss","password":"{PASSWORD}"}}"#),
    )
    .await;
    assert_eq!(code, 200, "{body}");
    let v: serde_json::Value =
        serde_json::from_str(body.trim_end_matches(|c: char| c.is_control() || c == '0'))
            .unwrap_or_else(|_| {
                // Chunked encoding: take the JSON object between the first '{' and the last '}'.
                let start = body.find('{').unwrap();
                let end = body.rfind('}').unwrap();
                serde_json::from_str(&body[start..=end]).unwrap()
            });
    let token = v["token"].as_str().unwrap().to_string();
    let (code, _) = request(&host, "GET", "/admin/api/overview", &token, "").await;
    assert_eq!(code, 200);
    let (code, _) = request(
        &host,
        "POST",
        "/admin/api/users/eve/ban",
        &token,
        r#"{"hours":2,"reason":"spam"}"#,
    )
    .await;
    assert_eq!(code, 200);
    // An administrator may change their own wallet, not ban themselves.
    let (code, body) = request(
        &host,
        "POST",
        "/admin/api/users/boss/coins",
        &token,
        r#"{"coins":500,"reason":"test"}"#,
    )
    .await;
    assert_eq!(code, 200, "{body}");
    let (code, body) =
        request(&host, "POST", "/admin/api/users/boss/vip", &token, r#"{"days":30,"reason":"t"}"#)
            .await;
    assert_eq!(code, 200, "{body}");
    let (code, _) =
        request(&host, "POST", "/admin/api/users/boss/ban", &token, r#"{"reason":"x"}"#).await;
    assert_eq!(code, 400, "nobody bans themselves");
    // A note on the board reaches everyone in the lounge at once.
    let (code, body) = request(
        &host,
        "POST",
        "/admin/api/board",
        &token,
        r#"{"text":"Cup on Saturday","days":3}"#,
    )
    .await;
    assert_eq!(code, 200, "{body}");
    let ServerMsg::Board { notes } = boss
        .expect("board", |m| matches!(m, ServerMsg::Board { notes } if !notes.is_empty()))
        .await
    else {
        unreachable!()
    };
    assert_eq!((notes[0].text.as_str(), notes[0].author.as_str()), ("Cup on Saturday", "boss"));
    let (code, _) =
        request(&host, "DELETE", &format!("/admin/api/board/{}", notes[0].id), &token, "").await;
    assert_eq!(code, 200);
    boss.expect("empty board", |m| matches!(m, ServerMsg::Board { notes } if notes.is_empty()))
        .await;
    // Eve can't sign in while banned.
    let mut c = Client::connect(&url).await;
    c.send(ClientMsg::Login { username: "eve".into(), password: PASSWORD.into() }).await;
    let ServerMsg::AuthError { reason } =
        c.expect("banned", |m| matches!(m, ServerMsg::AuthError { .. })).await
    else {
        unreachable!()
    };
    assert!(reason.contains("banned"), "{reason}");
}

/// Signing in again from another place ends what the old connection had
/// going in the lounge: its open table must not stay behind as a ghost.
#[tokio::test]
async fn a_replaced_session_leaves_no_table_or_queue_behind() {
    let url = start(Rules::default()).await;
    let (mut alice, token) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    alice.send(ClientMsg::OpenTable { rated: false, password: None }).await;
    let ServerMsg::TableOpened { table } =
        bob.expect("table", |m| matches!(m, ServerMsg::TableOpened { .. })).await
    else {
        unreachable!()
    };
    // Alice opens the game on another computer.
    let mut alice2 = Client::connect(&url).await;
    alice2.send(ClientMsg::Resume { token }).await;
    alice2.expect("auth", |m| matches!(m, ServerMsg::AuthOk { .. })).await;
    bob.expect("closed", |m| matches!(m, ServerMsg::TableClosed { id } if *id == table.id)).await;
    let ServerMsg::Tables { tables } =
        alice2.expect("tables", |m| matches!(m, ServerMsg::Tables { .. })).await
    else {
        unreachable!()
    };
    assert!(tables.is_empty(), "the old table is gone");
    bob.send(ClientMsg::JoinTable { id: table.id, password: None }).await;
    bob.expect("error", |m| matches!(m, ServerMsg::Error { .. })).await;
}

/// Lounge actions that everyone hears about can't be used to flood the lounge.
#[tokio::test]
async fn lounge_actions_are_rate_limited() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    for _ in 0..40 {
        alice.send(ClientMsg::OpenTable { rated: false, password: None }).await;
    }
    alice.expect("limit", |m| matches!(m, ServerMsg::Error { .. })).await;
    alice.send(ClientMsg::Chat { text: "done".into() }).await;
    let mut opened = 0;
    while let ServerMsg::TableOpened { .. } = bob
        .expect("tables", |m| matches!(m, ServerMsg::TableOpened { .. } | ServerMsg::Chat { .. }))
        .await
    {
        opened += 1;
    }
    assert!((1..=14).contains(&opened), "{opened} tables announced");
    // Leaving a tournament one never joined says nothing to anyone.
    bob.send(ClientMsg::CreateTournament {
        name: "Friday".into(),
        size: 4,
        rated: false,
        password: None,
        entry_fee: 0,
        format: Default::default(),
    })
    .await;
    let ServerMsg::TournamentUpdated { tournament } =
        bob.expect("tournament", |m| matches!(m, ServerMsg::TournamentUpdated { .. })).await
    else {
        unreachable!()
    };
    let (mut carol, _) = Client::register(&url, "carol").await;
    carol.send(ClientMsg::LeaveTournament { id: tournament.id }).await;
    carol.send(ClientMsg::Chat { text: "hello".into() }).await;
    let next = bob
        .expect("next", |m| {
            matches!(m, ServerMsg::TournamentUpdated { .. } | ServerMsg::Chat { .. })
        })
        .await;
    assert!(matches!(next, ServerMsg::Chat { .. }), "{next:?}");
}

/// A server on its own port over `store`; returns its address and handle.
async fn start_on(store: snookered_server::store::Store) -> (String, std::sync::Arc<Server>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let rules = Rules { max_connections_per_ip: 100, ..Rules::default() };
    let config = Config { bind: addr, rules, ..Config::default() };
    let server = Server::with_store(config, store).await;
    tokio::spawn(serve(listener, server.clone()));
    (format!("ws://{addr}/ws"), server)
}

impl Client {
    async fn login(url: &str, name: &str, password: &str) -> Client {
        let mut c = Client::connect(url).await;
        c.send(ClientMsg::Login { username: name.into(), password: password.into() }).await;
        c.expect("auth", |m| matches!(m, ServerMsg::AuthOk { .. })).await;
        c
    }

    /// Waits until the server closes the connection.
    async fn closed(&mut self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, self.ws.next()).await {
                Err(_) => panic!("the connection stayed open"),
                Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => return,
                Ok(Some(Ok(_))) => {}
            }
        }
    }
}

/// Plays `n` shots of a game (AI on both sides); returns the game after them.
async fn play_shots(
    a: &mut Client,
    b: &mut Client,
    mut game: EightBallGame,
    sa: usize,
    n: usize,
) -> EightBallGame {
    for k in 0..n {
        if game.game_over().is_some() {
            break;
        }
        let d = ai::decide(&game, Difficulty::Hard, k as u64);
        let turn = game.shots_taken();
        let msg = ClientMsg::Shoot {
            turn,
            shot: d.shot,
            called_pocket: d.called_pocket,
            cue_ball: d.cue_ball,
        };
        if game.current_player() == sa {
            a.send(msg).await
        } else {
            b.send(msg).await
        }
        let ServerMsg::ShotResult { end, .. } =
            a.expect("result", |m| matches!(m, ServerMsg::ShotResult { .. })).await
        else {
            unreachable!()
        };
        b.expect("result b", |m| matches!(m, ServerMsg::ShotResult { .. })).await;
        game = end;
    }
    game
}

#[tokio::test]
async fn a_restart_hands_the_game_to_the_next_server() {
    let store = snookered_server::store::Store::memory();
    let (url_a, server_a) = start_on(store.share().unwrap()).await;
    let (mut alice, _) = Client::register(&url_a, "alice").await;
    let (mut bob, _) = Client::register(&url_a, "bob").await;
    let (game, sa, _) = start_match(&mut alice, &mut bob, "bob", "alice", true).await;
    let game = play_shots(&mut alice, &mut bob, game, sa, 3).await;
    assert!(game.game_over().is_none());

    // The first server is told to stop: no new games, a notice, and after
    // the drain time the game is handed over (the connections close).
    let drain = tokio::spawn(snookered_server::drain(server_a.clone(), 1));
    alice.expect("draining", |m| matches!(m, ServerMsg::Draining { .. })).await;
    alice.closed().await;
    bob.closed().await;
    drain.await.unwrap();

    // The next server rebuilds the table when the players come back.
    let (url_b, _server_b) = start_on(store).await;
    let mut alice = Client::login(&url_b, "alice", PASSWORD).await;
    let ServerMsg::MatchStarted { game: rebuilt, seat, rated, .. } =
        alice.expect("game back", |m| matches!(m, ServerMsg::MatchStarted { .. })).await
    else {
        unreachable!()
    };
    assert_eq!((seat, rated), (sa, true));
    assert_eq!(rebuilt, game, "the same table, ball for ball");
    let mut bob = Client::login(&url_b, "bob", PASSWORD).await;
    bob.expect("game back", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
    // And the game goes on to the end.
    let result = play_out(&mut alice, &mut bob, rebuilt, sa).await;
    assert!(result.rated);
}

#[tokio::test]
async fn account_password_recovery_email_and_deletion() {
    let url = start(Rules::default()).await;
    let (mut carol, _) = Client::register(&url, "carol").await;
    carol.send(ClientMsg::ChangePassword { old: "wrong-one".into(), new: "another1".into() }).await;
    carol.expect("wrong", |m| matches!(m, ServerMsg::Error { .. })).await;
    carol.send(ClientMsg::ChangePassword { old: PASSWORD.into(), new: "another1".into() }).await;
    carol.expect("changed", |m| matches!(m, ServerMsg::Notice { .. })).await;
    let mut again = Client::login(&url, "carol", "another1").await;
    again.send(ClientMsg::SetRecoveryEmail { email: "not an email".into() }).await;
    again.expect("bad e-mail", |m| matches!(m, ServerMsg::Error { .. })).await;
    again.send(ClientMsg::SetRecoveryEmail { email: " Carol@Example.com ".into() }).await;
    let ServerMsg::Account { recovery_email, .. } =
        again.expect("account", |m| matches!(m, ServerMsg::Account { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(recovery_email.as_deref(), Some("carol@example.com"));

    let (mut dave, _) = Client::register(&url, "dave").await;
    again.send(ClientMsg::DeleteAccount { password: "another1".into() }).await;
    again.expect("deleted", |m| matches!(m, ServerMsg::AccountDeleted)).await;
    again.closed().await;
    // The name is free of the old account: it can't sign in, and its profile is gone.
    let mut c = Client::connect(&url).await;
    c.send(ClientMsg::Login { username: "carol".into(), password: "another1".into() }).await;
    c.expect("refused", |m| matches!(m, ServerMsg::AuthError { .. })).await;
    dave.send(ClientMsg::Profile { username: "carol".into() }).await;
    dave.expect("gone", |m| matches!(m, ServerMsg::Error { .. })).await;
}

#[tokio::test]
async fn replays_coins_featured_table_and_codes() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    alice.expect("daily bonus", |m| matches!(m, ServerMsg::CoinsEarned { .. })).await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    let (mut eve, _) = Client::register(&url, "eve").await;

    // An invitation code: "snookered://join/abc123" works as well as "ABC123".
    alice.send(ClientMsg::CreateInviteCode { rated: true }).await;
    let ServerMsg::InviteCode { code } =
        alice.expect("code", |m| matches!(m, ServerMsg::InviteCode { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(code.len(), 6);
    bob.send(ClientMsg::JoinByCode { code: format!("snookered://join/{}", code.to_lowercase()) })
        .await;
    let ServerMsg::MatchStarted { game, seat: sa, match_id, table_look, .. } =
        alice.expect("match", |m| matches!(m, ServerMsg::MatchStarted { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(table_look[0], "cloth_green", "the host's table");
    bob.expect("match b", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;

    // Anyone can watch the featured game.
    let ServerMsg::Featured { featured } =
        eve.expect("featured", |m| matches!(m, ServerMsg::Featured { featured: Some(_) })).await
    else {
        unreachable!()
    };
    assert_eq!(featured.unwrap().match_id, match_id);
    eve.send(ClientMsg::WatchMatch { match_id }).await;
    let ServerMsg::MatchStarted { spectating, .. } =
        eve.expect("watching", |m| matches!(m, ServerMsg::MatchStarted { .. })).await
    else {
        unreachable!()
    };
    assert!(spectating);

    let result = play_out(&mut alice, &mut bob, game, sa).await;
    // Coins for a rated game played to the end.
    let ServerMsg::CoinsEarned { amount, .. } =
        alice.expect("coins", |m| matches!(m, ServerMsg::CoinsEarned { .. })).await
    else {
        unreachable!()
    };
    let alice_won = result.winner == sa;
    use snookered_server::account::{COINS_PER_GAME, COINS_PER_WIN};
    let expected = COINS_PER_GAME + if alice_won { COINS_PER_WIN } else { 0 };
    assert_eq!(amount as i64, expected);
    // The game moved the missions on: one game played, one won (or not).
    let ServerMsg::Missions { daily, weekly, day_ends, week_ends } =
        alice.expect("missions", |m| matches!(m, ServerMsg::Missions { .. })).await
    else {
        unreachable!()
    };
    assert_eq!((daily.len(), weekly.len()), (3, 2));
    assert!(day_ends > snookered_server::now() && week_ends >= day_ends);
    for m in daily.iter().chain(&weekly) {
        let goal = snookered_proto::missions::mission(&m.code).unwrap().goal;
        use snookered_proto::missions::Goal;
        match goal {
            Goal::Play => assert_eq!(m.progress, 1, "{}", m.code),
            Goal::Win => assert_eq!(m.progress, alice_won as u32, "{}", m.code),
            _ => {}
        }
    }

    // The game can be watched again, stroke by stroke.
    alice.send(ClientMsg::Replays).await;
    let ServerMsg::Replays { games, limit } =
        alice.expect("replays", |m| matches!(m, ServerMsg::Replays { .. })).await
    else {
        unreachable!()
    };
    assert_eq!((games.len(), limit), (1, 10));
    alice.send(ClientMsg::Replay { match_id: games[0].match_id }).await;
    let ServerMsg::Replay { replay } =
        alice.expect("replay", |m| matches!(m, ServerMsg::Replay { .. })).await
    else {
        unreachable!()
    };
    let (end, _) = replay.replay.play(None).unwrap();
    assert_eq!(end.game_over().map(|o| o.winner), Some(result.winner));
    // Only the players (and staff) may.
    eve.send(ClientMsg::Replay { match_id: games[0].match_id }).await;
    eve.expect("not hers", |m| matches!(m, ServerMsg::Error { .. })).await;

    // The profile has the rating chart and the career numbers.
    eve.send(ClientMsg::Profile { username: "alice".into() }).await;
    let ServerMsg::Profile { profile } =
        eve.expect("profile", |m| matches!(m, ServerMsg::Profile { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(profile.rating_history.len(), 1);
    assert_eq!(profile.career.games, 1);
    assert!(profile.career.shots > 0);
}

#[tokio::test]
async fn arenas_with_an_entry_fee() {
    let url = start(Rules::default()).await;
    let (mut alice, _) = Client::register(&url, "alice").await;
    let (mut bob, _) = Client::register(&url, "bob").await;
    // The daily bonus (20 coins) pays the entry.
    for c in [&mut alice, &mut bob] {
        c.expect("daily bonus", |m| matches!(m, ServerMsg::CoinsEarned { .. })).await;
    }
    alice
        .send(ClientMsg::CreateTournament {
            name: "Blitz".into(),
            size: 4,
            rated: false,
            password: None,
            entry_fee: 1000,
            format: snookered_proto::TournamentFormat::Arena,
        })
        .await;
    alice.expect("too dear", |m| matches!(m, ServerMsg::Error { .. })).await;
    alice
        .send(ClientMsg::CreateTournament {
            name: "Blitz".into(),
            size: 4,
            rated: false,
            password: None,
            entry_fee: 10,
            format: snookered_proto::TournamentFormat::Arena,
        })
        .await;
    let ServerMsg::TournamentUpdated { tournament } =
        bob.expect("created", |m| matches!(m, ServerMsg::TournamentUpdated { .. })).await
    else {
        unreachable!()
    };
    assert_eq!((tournament.entry_fee, tournament.prizes), (10, [7, 2]));
    bob.send(ClientMsg::JoinTournament { id: tournament.id, password: None }).await;
    let ServerMsg::TournamentUpdated { tournament } = alice
        .expect("joined", |m| {
            matches!(m, ServerMsg::TournamentUpdated { tournament } if tournament.players.len() == 2)
        })
        .await
    else {
        unreachable!()
    };
    assert_eq!(tournament.prizes, [14, 4], "the pot grows");
    alice.send(ClientMsg::StartTournament { id: tournament.id }).await;
    alice.expect("paired", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
    bob.expect("paired", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
    alice.send(ClientMsg::Resign).await;
    let ServerMsg::TournamentUpdated { tournament } = alice
        .expect("standings", |m| {
            matches!(m, ServerMsg::TournamentUpdated { tournament } if tournament.standings.iter().any(|s| s.points > 0))
        })
        .await
    else {
        unreachable!()
    };
    assert_eq!(tournament.standings[0].username, "bob");
    assert_eq!((tournament.standings[0].points, tournament.standings[0].streak), (2, 1));
    assert!(tournament.ends_at.is_some());
}

#[tokio::test]
async fn withdrawing_while_waiting_for_an_opponent_frees_the_player() {
    let url = start(Rules::default()).await;
    let (mut ana, _) = Client::register(&url, "ana").await;
    let (mut ben, _) = Client::register(&url, "ben").await;
    let (mut cid, _) = Client::register(&url, "cid").await;
    ana.send(ClientMsg::CreateTournament {
        name: "Three".into(),
        size: 4,
        rated: false,
        password: None,
        entry_fee: 0,
        format: Default::default(),
    })
    .await;
    let ServerMsg::TournamentUpdated { tournament } =
        ben.expect("created", |m| matches!(m, ServerMsg::TournamentUpdated { .. })).await
    else {
        unreachable!()
    };
    let id = tournament.id;
    for c in [&mut ben, &mut cid] {
        c.send(ClientMsg::JoinTournament { id, password: None }).await;
    }
    cid.expect("joined", |m| {
        matches!(m, ServerMsg::TournamentUpdated { tournament } if tournament.players.len() == 3)
    })
    .await;
    // Three players: the top seed (ana, first by name) waits in the final.
    ana.send(ClientMsg::StartTournament { id }).await;
    ben.expect("semi-final", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
    cid.expect("semi-final", |m| matches!(m, ServerMsg::MatchStarted { .. })).await;
    // Ana withdraws before her opponent is known: she may play elsewhere.
    ana.send(ClientMsg::LeaveTournament { id }).await;
    ana.send(ClientMsg::OpenTable { rated: false, password: None }).await;
    ana.expect("table", |m| {
        assert!(!matches!(m, ServerMsg::Error { .. }), "still held by the tournament: {m:?}");
        matches!(m, ServerMsg::TableOpened { .. })
    })
    .await;
    // The semi-final's winner takes the final by walkover.
    ben.send(ClientMsg::Resign).await;
    let ServerMsg::TournamentUpdated { tournament } = cid
        .expect("finished", |m| {
            matches!(m, ServerMsg::TournamentUpdated { tournament }
                if tournament.state == TournamentState::Finished)
        })
        .await
    else {
        unreachable!()
    };
    assert_eq!(tournament.champion.as_deref(), Some("cid"));
}
