//! End-to-end tests: real WebSocket clients against a server on a random port.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use snook_core::{Difficulty, EightBallGame, ai};
use snook_proto::{
    ClientMsg, PROTOCOL_VERSION, ReportReason, ServerMsg, TournamentState, parse_server,
};
use snook_server::lounge::Rules;
use snook_server::{Config, Server, serve};
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
) -> snook_proto::MatchResult {
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
        |m| matches!(m, ServerMsg::Reaction { seat, .. } if *seat == snook_proto::SPECTATOR_SEAT),
    )
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
    let (_boss, _) = Client::register(&url, "boss").await;
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
