//! The storage back ends behave the same. The in-memory store is always
//! tested; PostgreSQL when `SNOOK_TEST_DATABASE_URL` points at an empty
//! database (it is wiped first), e.g.
//! `SNOOK_TEST_DATABASE_URL=postgres://snook:snook@localhost/snook_test`.

use snook_core::stats::ShotStats;
use snook_core::{GameOverReason, Shot};
use snook_proto::Role;
use snook_proto::replay::{Replay, Stroke};
use snook_server::store::{
    BuyOutcome, FriendOutcome, FriendState, LiveMatch, MatchRecord, NewReport, NewUser, Payment,
    SessionKind, Store, TournamentRecord, day_of,
};

fn new_user<'a>(name: &'a str) -> NewUser<'a> {
    NewUser {
        username: name,
        password_hash: "h",
        country: "BR",
        rating: 1200,
        role: Role::Player,
        ip: "1.2.3.4",
    }
}

async fn exercise(store: Store) {
    let t = 1_700_000_000;
    let a = store.create_user(&new_user("Alice")).await.unwrap().unwrap();
    assert!(store.create_user(&new_user("alice")).await.unwrap().is_none(), "case-insensitive");
    let b = store.create_user(&new_user("bob")).await.unwrap().unwrap();
    assert_eq!(store.user_by_name("ALICE").await.unwrap().unwrap().id, a.id);

    // Sessions.
    store.create_session("hash1", a.id, SessionKind::Game, t, t + 100).await.unwrap();
    assert!(store.session_user("hash1", SessionKind::Admin, t).await.unwrap().is_none());
    let (u, created) = store.session_user("hash1", SessionKind::Game, t).await.unwrap().unwrap();
    assert_eq!((u.id, created), (a.id, t));
    assert!(store.session_user("hash1", SessionKind::Game, t + 200).await.unwrap().is_none());
    assert_eq!(store.purge_sessions(t + 200).await.unwrap(), 1);

    // A rated match.
    let m = MatchRecord {
        players: [a.id, b.id],
        winner: 0,
        reason: GameOverReason::PocketedEight,
        rated: true,
        delta: [16, -16],
        ratings_after: [1216, 1184],
        shots: 30,
        tournament: None,
        eight_win: true,
        runout: true,
        shutout: false,
        ended_at: t,
        replay: Some(Replay {
            seed: u64::MAX - 5,
            breaker: 1,
            strokes: vec![
                Stroke::Shot {
                    shot: Shot::new(0.25, 3.5),
                    called: None,
                    cue_ball: None,
                    timeout: false,
                },
                Stroke::TimeFoul,
            ],
        }),
        stats: [
            ShotStats {
                shots: 10,
                potting_shots: 6,
                balls: 7,
                fouls: 1,
                best_run: 4,
                ..Default::default()
            },
            ShotStats {
                shots: 8,
                potting_shots: 2,
                balls: 2,
                fouls: 2,
                best_run: 1,
                ..Default::default()
            },
        ],
        abandoned: None,
    };
    let ([ua, ub], match_id) = store.record_match(&m).await.unwrap();
    let (ua, ub) = (ua.unwrap(), ub.unwrap());
    assert_eq!((ua.rating, ua.peak_rating, ua.games, ua.wins, ua.streak), (1216, 1216, 1, 1, 1));
    assert_eq!((ua.eight_wins, ua.runouts, ua.shutouts), (1, 1, 0));
    assert_eq!((ub.rating, ub.peak_rating, ub.games, ub.wins, ub.streak), (1184, 1200, 1, 0, 0));
    let casual = MatchRecord {
        rated: false,
        delta: [0, 0],
        winner: 1,
        replay: None,
        stats: Default::default(),
        abandoned: Some(0),
        ..m.clone()
    };
    let ([ua, ub], _) = store.record_match(&casual).await.unwrap();
    let ua = ua.unwrap();
    assert_eq!((ua.casual_games, ua.abandons), (1, 1));
    // Feats only count in rated games.
    assert_eq!(ub.unwrap().eight_wins, 0);

    // Replays, end-of-game numbers and the rating chart.
    let r = store.replay(match_id).await.unwrap().expect("replay kept");
    assert_eq!(r.replay, m.replay.clone().unwrap());
    assert_eq!((r.players[0].as_str(), r.ratings, r.winner), ("Alice", [1200, 1200], 0));
    assert!(store.replay(match_id + 1).await.unwrap().is_none(), "the casual game has none");
    let c = store.career(a.id).await.unwrap();
    assert_eq!((c.games, c.shots, c.balls, c.best_run, c.pot_rate()), (1, 10, 7, 4, 60));
    assert_eq!(store.rating_history(a.id, 10).await.unwrap(), vec![(t, 1216)]);

    let history = store.history(b.id, 10).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        (history[1].match_id, history[1].replay, history[0].replay),
        (match_id, true, false)
    );
    assert_eq!(history[1].opponent, "Alice");
    assert_eq!((history[1].won, history[1].delta, history[1].rating_after), (false, -16, 1184));
    assert_eq!(history[1].reason, "pocketed_eight");

    let lb = store.leaderboard(10, t).await.unwrap();
    assert_eq!(lb[0].username, "Alice");
    assert_eq!(lb[0].country, "BR");
    assert!(!lb[0].vip);
    assert_eq!(store.rank_of(b.id).await.unwrap(), Some(2));

    // The shop: credits are applied once per reference, purchases never
    // go below zero, refunds are recorded once.
    assert_eq!(store.wallet(a.id).await.unwrap().coins, 0);
    assert_eq!(
        store.buy_item(a.id, "cloth_blue", 400, t).await.unwrap(),
        BuyOutcome::NotEnoughCoins
    );
    assert!(store.credit(a.id, 500, "pack", Some("pay:pi_1"), t).await.unwrap());
    assert!(!store.credit(a.id, 500, "pack", Some("pay:pi_1"), t).await.unwrap(), "same reference");
    assert_eq!(store.wallet(a.id).await.unwrap().coins, 500);
    assert_eq!(store.buy_item(a.id, "cloth_blue", 400, t).await.unwrap(), BuyOutcome::Bought);
    assert_eq!(store.buy_item(a.id, "cloth_blue", 400, t).await.unwrap(), BuyOutcome::AlreadyOwned);
    store.equip(a.id, "cloth", "cloth_blue").await.unwrap();
    store.equip(a.id, "cloth", "cloth_green").await.unwrap();
    let w = store.wallet(a.id).await.unwrap();
    assert_eq!((w.coins, w.owned.clone()), (100, vec!["cloth_blue".to_string()]));
    assert_eq!(w.equipped.get("cloth").map(String::as_str), Some("cloth_green"));
    store.set_vip(a.id, t + 100, false).await.unwrap();
    store.set_vip(a.id, t + 50, false).await.unwrap();
    assert_eq!(store.wallet(a.id).await.unwrap().vip_until, t + 100, "never shortened");
    store.set_vip(a.id, t, true).await.unwrap();
    assert_eq!(store.wallet(a.id).await.unwrap().vip_until, t);
    store.set_stripe_customer(a.id, "cus_1").await.unwrap();
    assert_eq!(store.user_by_stripe_customer("cus_1").await.unwrap(), Some(a.id));
    let p = Payment {
        id: "pi_1".into(),
        user_id: a.id,
        kind: "coins".into(),
        coins: 500,
        amount: 490,
        currency: "brl".into(),
        at: t,
        refunded: false,
        email: Some("alice@example.com".into()),
    };
    assert!(store.record_payment(&p).await.unwrap());
    assert!(!store.record_payment(&p).await.unwrap());
    assert!(store.refund_payment("pi_1").await.unwrap());
    assert!(!store.refund_payment("pi_1").await.unwrap());
    assert!(store.payment("pi_1").await.unwrap().unwrap().refunded);
    assert!(store.grant_item(a.id, "cue_ebony", "staff", t).await.unwrap());
    assert!(!store.grant_item(a.id, "cue_ebony", "staff", t).await.unwrap());
    assert_eq!(store.ledger(a.id, 10).await.unwrap().len(), 2);
    assert_eq!(store.payments_of(a.id, 10).await.unwrap().len(), 1);
    assert_eq!(store.payment_email(a.id).await.unwrap().as_deref(), Some("alice@example.com"));
    store.set_vip(a.id, t + 1000, false).await.unwrap();
    assert!(store.leaderboard(10, t).await.unwrap()[0].vip, "the crown shows in the ranking");

    // Coins for playing: the daily bonus once a day, games up to a cap.
    let day = day_of(t);
    let before = store.wallet(a.id).await.unwrap().coins;
    assert!(store.claim_daily(a.id, day, 20).await.unwrap());
    assert!(!store.claim_daily(a.id, day, 20).await.unwrap());
    assert_eq!(store.earn(a.id, day, 25, 40, "win", t).await.unwrap(), 25);
    assert_eq!(store.earn(a.id, day, 25, 40, "win", t).await.unwrap(), 15, "capped");
    assert_eq!(store.earn(a.id, day, 25, 40, "win", t).await.unwrap(), 0);
    assert_eq!(store.earn(a.id, day + 1, 25, 40, "win", t).await.unwrap(), 25, "a new day");
    assert_eq!(store.wallet(a.id).await.unwrap().coins, before + 20 + 25 + 15 + 25);
    // Spending and gifts never go below zero.
    let coins = store.wallet(a.id).await.unwrap().coins;
    assert!(!store.spend(a.id, coins + 1, "fee", Some("tour:1:1"), t).await.unwrap());
    assert!(store.spend(a.id, 10, "fee", Some("tour:1:1"), t).await.unwrap());
    assert_eq!(
        store.gift_item(a.id, b.id, "chalk_red", 1_000_000, t).await.unwrap(),
        BuyOutcome::NotEnoughCoins
    );
    assert_eq!(store.gift_item(a.id, b.id, "chalk_red", 150, t).await.unwrap(), BuyOutcome::Bought);
    assert_eq!(
        store.gift_item(a.id, b.id, "chalk_red", 150, t).await.unwrap(),
        BuyOutcome::AlreadyOwned
    );
    assert!(store.wallet(b.id).await.unwrap().owned.contains(&"chalk_red".to_string()));
    assert_eq!(store.wallet(a.id).await.unwrap().coins, coins - 10 - 150);

    // Games in progress survive a restart.
    let live = LiveMatch {
        id: 1 << 40,
        players: [a.id, b.id],
        rated: true,
        replay: m.replay.clone().unwrap(),
        updated_at: t,
    };
    store.save_live(&live).await.unwrap();
    let mut later = live.clone();
    later.replay.strokes.push(Stroke::Resign { seat: 0 });
    later.updated_at = t + 5;
    store.save_live(&later).await.unwrap();
    assert_eq!(store.live_match_of(b.id).await.unwrap(), Some(later.clone()));
    assert_eq!(store.purge_live(t).await.unwrap(), 0);
    store.delete_live(live.id).await.unwrap();
    assert!(store.live_match_of(a.id).await.unwrap().is_none());

    // Aim statistics.
    store.add_shot_stats(a.id, &[(14, 10, 9), (2, 5, 5)]).await.unwrap();
    store.add_shot_stats(a.id, &[(14, 10, 8)]).await.unwrap();
    let rows = store.aim_rows(&[11, 14], 5).await.unwrap();
    assert_eq!((rows.len(), rows[0].attempts, rows[0].made), (1, 20, 17));
    assert!(store.aim_rows(&[11, 14], 50).await.unwrap().is_empty());

    // Activity and metrics.
    store.mark_active(a.id, day).await.unwrap();
    store.mark_active(a.id, day).await.unwrap();
    store.mark_active(b.id, day).await.unwrap();
    let metrics = store.metrics(t).await.unwrap();
    assert_eq!(metrics.days.len(), 30);
    assert_eq!(metrics.days.last().unwrap().1, 2);

    // Achievements.
    let new = store.add_achievements(a.id, &["wins_1", "games_1"], t).await.unwrap();
    assert_eq!(new.len(), 2);
    let new = store.add_achievements(a.id, &["wins_1", "eight_10"], t).await.unwrap();
    assert_eq!(new, vec!["eight_10".to_string()]);
    assert_eq!(store.achievements(a.id).await.unwrap().len(), 3);

    // Friends.
    assert_eq!(store.friend_request(a.id, b.id, t).await.unwrap(), FriendOutcome::Sent);
    assert_eq!(store.friend_request(a.id, b.id, t).await.unwrap(), FriendOutcome::AlreadySent);
    let fb = store.friends(b.id).await.unwrap();
    assert_eq!(fb[0].1, FriendState::Incoming);
    assert!(store.friend_accept(b.id, a.id).await.unwrap());
    assert_eq!(store.friend_count(a.id).await.unwrap(), 1);
    assert_eq!(store.friends(a.id).await.unwrap()[0].1, FriendState::Friends);
    store.friend_remove(b.id, a.id).await.unwrap();
    assert_eq!(store.friend_count(a.id).await.unwrap(), 0);
    // Asking someone who already asked you makes you friends.
    store.friend_request(b.id, a.id, t).await.unwrap();
    assert_eq!(store.friend_request(a.id, b.id, t).await.unwrap(), FriendOutcome::Accepted);

    // Blocks, reports, staff.
    store.block(a.id, b.id).await.unwrap();
    assert_eq!(store.blocked(a.id).await.unwrap()[0].id, b.id);
    store.unblock(a.id, b.id).await.unwrap();
    assert!(store.blocked(a.id).await.unwrap().is_empty());
    store
        .create_report(&NewReport {
            reporter: a.id,
            target: b.id,
            reason: "Spam".into(),
            details: "d".into(),
            context: "c".into(),
            at: t,
        })
        .await
        .unwrap();
    let open = store.reports(Some(0), 10).await.unwrap();
    assert_eq!((open.len(), open[0].target.as_str()), (1, "bob"));
    assert!(store.resolve_report(open[0].id, a.id, 2, "ok", t).await.unwrap());
    assert!(store.reports(Some(0), 10).await.unwrap().is_empty());
    assert_eq!(store.reports_about(b.id, 10).await.unwrap().len(), 1);
    store.set_ban(b.id, Some(t + 50), Some("spam")).await.unwrap();
    assert!(store.user_by_id(b.id).await.unwrap().unwrap().is_banned(t));
    // Only a ban still running hides a player from the ranking (this one
    // ended long ago).
    assert!(store.leaderboard(10).await.unwrap().iter().any(|e| e.username == "bob"));
    store.set_ban(a.id, Some(i64::MAX), Some("cheating")).await.unwrap();
    assert!(!store.leaderboard(10).await.unwrap().iter().any(|e| e.username == "Alice"));
    assert_eq!(store.rank_of(b.id).await.unwrap(), Some(1));
    store.set_ban(a.id, None, None).await.unwrap();
    assert_eq!(store.rank_of(b.id).await.unwrap(), Some(2));
    store.set_mute(b.id, Some(t + 50)).await.unwrap();
    store.set_role(a.id, Role::Moderator).await.unwrap();
    assert_eq!(store.user_by_id(a.id).await.unwrap().unwrap().role, Role::Moderator);
    store.audit(a.id, "ban", "bob", "1h", t).await.unwrap();
    assert_eq!(store.audit_log(5).await.unwrap()[0].staff, "Alice");
    let (found, total) = store.search_users("o", 0, 10).await.unwrap();
    assert_eq!((found.len(), total), (1, 1));
    let (all, total) = store.search_users("", 0, 1).await.unwrap();
    assert_eq!((all.len(), total), (1, 2));
    let o = store.overview(t).await.unwrap();
    assert_eq!((o.users, o.matches_total), (2, 2));

    store
        .record_tournament(&TournamentRecord {
            name: "Cup".into(),
            host: a.id,
            size: 4,
            rated: false,
            players: vec![a.id, b.id],
            winner: Some(a.id),
            finished_at: t,
            entry_fee: 50,
            prizes: "Alice:70".into(),
        })
        .await
        .unwrap();
    let ua = store.user_by_id(a.id).await.unwrap().unwrap();
    assert_eq!((ua.tournaments_played, ua.tournaments_won), (1, 1));
    store.set_country(a.id, "PT").await.unwrap();
    assert_eq!(store.user_by_id(a.id).await.unwrap().unwrap().country, "PT");

    // Recovery e-mail, then deleting the account: it is anonymised and the
    // opponent keeps the games.
    store.set_recovery_email(b.id, Some("bob@example.com")).await.unwrap();
    let (found, _) = store.search_users("bob@example.com", 0, 10).await.unwrap();
    assert_eq!(found[0].id, b.id);
    store.create_session("hash_b", b.id, SessionKind::Game, t, t + 100).await.unwrap();
    store.delete_account(b.id, t).await.unwrap();
    let gone = store.user_by_id(b.id).await.unwrap().unwrap();
    assert_eq!(gone.username, format!("deleted-{}", b.id));
    assert!(
        gone.deleted_at.is_some() && gone.recovery_email.is_none() && gone.password_hash.is_empty()
    );
    assert!(store.session_user("hash_b", SessionKind::Game, t).await.unwrap().is_none());
    assert!(store.user_by_name("bob").await.unwrap().is_none());
    let history = store.history(a.id, 10).await.unwrap();
    assert_eq!(history.len(), 2, "the other player's history stays");
    assert_eq!(history[0].opponent, format!("deleted-{}", b.id));
    assert!(store.leaderboard(10, t).await.unwrap().iter().all(|e| e.username != gone.username));
}

#[tokio::test]
async fn memory_store() {
    exercise(Store::memory()).await;
}

#[tokio::test]
async fn postgres_store() {
    let Ok(url) = std::env::var("SNOOK_TEST_DATABASE_URL") else {
        eprintln!("SNOOK_TEST_DATABASE_URL not set: skipping");
        return;
    };
    // Start from an empty database.
    {
        use sqlx::Executor;
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        pool.execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    }
    exercise(Store::postgres(&url, 3).await.unwrap()).await;
}
