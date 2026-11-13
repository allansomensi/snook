//! Daily and weekly missions (the list is in `snookered_proto::missions`):
//! progress from rated games that pay coins, and the coins a finished
//! mission brings.

use snookered_core::stats::ShotStats;
use snookered_proto::ServerMsg;
use snookered_proto::missions::{self, Mission, MissionState};

use crate::store::User;
use crate::{Server, now};

/// Today's and this week's missions with `user`'s progress.
pub async fn message(server: &Server, user: &User) -> ServerMsg {
    let t = now();
    let (daily, weekly) = (missions::daily(t), missions::weekly(t));
    let periods: Vec<String> =
        daily.iter().chain(&weekly).map(|m| missions::period(m, t)).collect();
    let rows = server.store.mission_progress(user.id, &periods).await.unwrap_or_default();
    let state = |list: &[&Mission]| -> Vec<MissionState> {
        list.iter()
            .map(|m| {
                let period = missions::period(m, t);
                let progress = rows
                    .iter()
                    .find(|(p, c, _)| *p == period && c == m.code)
                    .map_or(0, |(_, _, n)| *n);
                MissionState { code: m.code.to_string(), progress }
            })
            .collect()
    };
    ServerMsg::Missions {
        daily: state(&daily),
        weekly: state(&weekly),
        day_ends: missions::day_ends(t),
        week_ends: missions::week_ends(t),
    }
}

/// Sends `user` their missions (when online).
pub async fn push(server: &Server, user: &User) {
    if server.config.rules.lan {
        return;
    }
    let msg = message(server, user).await;
    server.with(|l| l.notify(&user.username, msg));
}

/// A rated game that pays coins is over: `user` won it or not, with these
/// numbers. Moves the missions on and pays the ones it finished.
pub async fn after_game(server: &Server, user: &User, won: bool, stats: &ShotStats) {
    let t = now();
    for m in missions::daily(t).into_iter().chain(missions::weekly(t)) {
        let amount = missions::counts(m.goal, won, stats);
        if amount == 0 {
            continue;
        }
        let period = missions::period(m, t);
        let ends = if m.weekly { missions::week_ends(t) } else { missions::day_ends(t) };
        let progress = match server
            .store
            .advance_mission(user.id, &period, m.code, amount, m.target, ends)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("mission {} of {}: {e}", m.code, user.username);
                continue;
            }
        };
        if progress < m.target {
            continue;
        }
        // Keyed by the mission and its period: paid once.
        let reference = format!("mission:{}:{period}:{}", user.id, m.code);
        let reason = format!("mission {}", m.code);
        match server.store.credit(user.id, m.reward as i64, &reason, Some(&reference), t).await {
            Ok(true) => server.with(|l| {
                l.notify(
                    &user.username,
                    ServerMsg::CoinsEarned { amount: m.reward, reason: "mission".into() },
                )
            }),
            Ok(false) => {}
            Err(e) => tracing::error!("paying mission {} to {}: {e}", m.code, user.username),
        }
    }
    push(server, user).await;
}
