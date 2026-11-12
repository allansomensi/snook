//! The staff panel: a small web app served by the game server at `/admin`,
//! and the JSON API it uses under `/admin/api`.
//!
//! Staff are ordinary accounts with the moderator or administrator role
//! (the first administrators come from `SNOOK_ADMINS`). They sign in with
//! their game password and get a separate, short-lived admin session sent
//! as a bearer token (never a cookie, so there is no CSRF surface). Every
//! action is written to the audit log.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use snook_proto::{Role, Tier, clean_text, valid_password};

use crate::store::{SessionKind, User};
use crate::{Server, new_token, now, token_hash};

const ADMIN_SESSION: i64 = 8 * 60 * 60;
const INDEX: &str = include_str!("../admin/index.html");
const SCRIPT: &str = include_str!("../admin/app.js");
const STYLE: &str = include_str!("../admin/app.css");

pub fn router() -> Router<Arc<Server>> {
    Router::new()
        .route("/admin", get(|| async { page(INDEX, "text/html; charset=utf-8") }))
        .route("/admin/", get(|| async { page(INDEX, "text/html; charset=utf-8") }))
        .route("/admin/app.js", get(|| async { page(SCRIPT, "text/javascript; charset=utf-8") }))
        .route("/admin/app.css", get(|| async { page(STYLE, "text/css; charset=utf-8") }))
        .route("/admin/api/login", post(login))
        .route("/admin/api/logout", post(logout))
        .route("/admin/api/me", get(me))
        .route("/admin/api/overview", get(overview))
        .route("/admin/api/live", get(live))
        .route("/admin/api/users", get(users))
        .route("/admin/api/users/{name}", get(user_detail))
        .route("/admin/api/users/{name}/{action}", post(user_action))
        .route("/admin/api/reports", get(reports))
        .route("/admin/api/reports/{id}", post(resolve_report))
        .route("/admin/api/audit", get(audit))
        .route("/admin/api/metrics", get(metrics))
        .route("/admin/api/aim", get(aim_review))
        .route("/admin/api/replay/{id}", get(replay))
        .route("/admin/api/announce", post(announce))
        .route("/admin/api/motd", post(motd))
        .layer(middleware::from_fn(security_headers))
}

fn page(body: &'static str, kind: &'static str) -> Response {
    ([(header::CONTENT_TYPE, kind)], body).into_response()
}

async fn security_headers(req: Request, next: Next) -> Response {
    let is_admin = req.uri().path().starts_with("/admin");
    let mut res = next.run(req).await;
    if is_admin {
        let h = res.headers_mut();
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; \
                 connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            ),
        );
        h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    }
    res
}

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn err(code: StatusCode, msg: impl Into<String>) -> ApiError {
    ApiError(code, msg.into())
}

impl From<crate::store::StoreError> for ApiError {
    fn from(e: crate::store::StoreError) -> Self {
        tracing::error!("admin api: {e}");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Database error.")
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

/// The signed-in staff member.
struct Staff {
    user: User,
}

async fn staff(server: &Server, headers: &HeaderMap) -> Result<Staff, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Sign in first."))?;
    let (user, _) = server
        .store
        .session_user(&token_hash(token.trim()), SessionKind::Admin, now())
        .await?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Your session expired."))?;
    if !user.role.is_staff() || user.is_banned(now()) {
        return Err(err(StatusCode::FORBIDDEN, "Staff only."));
    }
    Ok(Staff { user })
}

fn require_admin(s: &Staff) -> Result<(), ApiError> {
    if s.user.role == Role::Admin {
        Ok(())
    } else {
        Err(err(StatusCode::FORBIDDEN, "Administrators only."))
    }
}

async fn log(server: &Server, s: &Staff, action: &str, target: &str, details: &str) {
    if let Err(e) = server.store.audit(s.user.id, action, target, details, now()).await {
        tracing::error!("audit log: {e}");
    }
    tracing::info!(staff = %s.user.username, action, target, details, "staff action");
}

fn user_json(u: &User, online: bool) -> Value {
    json!({
        "id": u.id,
        "username": u.username,
        "country": u.country,
        "role": u.role,
        "rating": u.rating,
        "peak_rating": u.peak_rating,
        "tier": Tier::of(u.rating).name(),
        "games": u.games,
        "wins": u.wins,
        "casual_games": u.casual_games,
        "created_at": u.created_at,
        "last_seen": u.last_seen,
        "banned_until": u.banned_until,
        "ban_reason": u.ban_reason,
        "muted_until": u.muted_until,
        "online": online,
        "recovery_email": u.recovery_email,
        "deleted_at": u.deleted_at,
        "abandons": u.abandons,
    })
}

/// Active players, games, new accounts, retention, the funnel, revenue
/// and the queue: what to build next is a decision made on these.
async fn metrics(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult {
    staff(&server, &headers).await?;
    let m = server.store.metrics(now()).await?;
    let waits: Vec<u32> = server.with(|l| l.queue_waits.iter().copied().collect());
    let avg_wait =
        if waits.is_empty() { 0.0 } else { waits.iter().sum::<u32>() as f64 / waits.len() as f64 };
    let mut sorted = waits.clone();
    sorted.sort_unstable();
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
    Ok(Json(json!({
        "metrics": m,
        "queue": { "pairs": waits.len() / 2, "avg_wait_secs": avg_wait, "median_wait_secs": median },
        "draining": server.with(|l| l.is_draining()),
    })))
}

/// The aim-assistance review queue: players whose potting on hard shots
/// (thin cuts over long distances) is far above the players of their tier.
/// Nobody is banned by this: it is a list to look at, with the replays.
async fn aim_review(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult {
    staff(&server, &headers).await?;
    let hard: Vec<usize> =
        (0..snook_proto::replay::BANDS).filter(|b| snook_proto::replay::is_hard_band(*b)).collect();
    let rows = server.store.aim_rows(&hard, AIM_MIN_ATTEMPTS).await?;
    Ok(Json(json!({
        "min_attempts": AIM_MIN_ATTEMPTS,
        "flagged": flag_outliers(&rows),
        "players": rows.len(),
    })))
}

/// Hard shots a player must have taken before being compared.
const AIM_MIN_ATTEMPTS: u32 = 40;

/// Players above the 99th percentile of their tier on hard shots (and with
/// a rate no honest player keeps up), most suspicious first.
pub fn flag_outliers(rows: &[crate::store::AimRow]) -> Vec<Value> {
    use std::collections::BTreeMap;
    let rate = |r: &crate::store::AimRow| r.made as f64 / r.attempts.max(1) as f64;
    let mut by_tier: BTreeMap<Tier, Vec<f64>> = BTreeMap::new();
    for r in rows {
        by_tier.entry(Tier::of(r.rating)).or_default().push(rate(r));
    }
    for v in by_tier.values_mut() {
        v.sort_by(f64::total_cmp);
    }
    let p99 = |v: &Vec<f64>| {
        v[((v.len() as f64 * 0.99).ceil() as usize).saturating_sub(1).min(v.len() - 1)]
    };
    let mut out: Vec<(f64, Value)> = rows
        .iter()
        .filter_map(|r| {
            let tier = Tier::of(r.rating);
            let rates = &by_tier[&tier];
            let mean = rates.iter().sum::<f64>() / rates.len() as f64;
            let sd =
                (rates.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / rates.len() as f64).sqrt();
            let x = rate(r);
            // With few players in a tier the percentile is the player
            // themself: compare with the tier's spread and a hard floor.
            let threshold = if rates.len() >= 100 { p99(rates) } else { mean + 3.0 * sd.max(0.05) };
            (x >= threshold.max(0.45) && x > mean).then(|| {
                let z = (x - mean) / sd.max(0.02);
                (
                    z,
                    json!({
                        "username": r.username, "rating": r.rating, "tier": tier.name(),
                        "attempts": r.attempts, "made": r.made, "rate": x,
                        "tier_mean": mean, "tier_players": rates.len(), "z": z,
                    }),
                )
            })
        })
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// A stored game (to download, or to watch in the game: staff can open any
/// replay from the player's profile).
async fn replay(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult {
    staff(&server, &headers).await?;
    let r = server
        .store
        .replay(id)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "No replay for that match."))?;
    Ok(Json(json!({
        "id": r.id, "players": r.players, "ratings": r.ratings, "winner": r.winner,
        "rated": r.rated, "at": r.at, "seed": r.replay.seed, "breaker": r.replay.breaker,
        "strokes": r.replay.strokes,
    })))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(
    State(server): State<Arc<Server>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> ApiResult {
    let ip = server.client_ip(peer, &headers).to_string();
    if !server.auth().may_try(&ip, &body.username) {
        return Err(err(StatusCode::TOO_MANY_REQUESTS, "Too many attempts. Wait a few minutes."));
    }
    let user = server.store.user_by_name(body.username.trim()).await?;
    let ok = match &user {
        Some(u) => server.verify_password(&body.password, &u.password_hash).await,
        None => false,
    };
    let user = match user {
        Some(u) if ok => u,
        _ => {
            server.auth().failed(&ip, &body.username);
            return Err(err(StatusCode::UNAUTHORIZED, "Wrong username or password."));
        }
    };
    let user = crate::promote_if_admin(&server, user).await;
    if !user.role.is_staff() || user.is_banned(now()) {
        server.auth().failed(&ip, &body.username);
        return Err(err(StatusCode::FORBIDDEN, "This account has no access to the panel."));
    }
    server.auth().succeeded(&body.username);
    let token = new_token();
    let t = now();
    server
        .store
        .create_session(&token_hash(&token), user.id, SessionKind::Admin, t, t + ADMIN_SESSION)
        .await?;
    let s = Staff { user };
    log(&server, &s, "login", &s.user.username, &ip).await;
    Ok(Json(json!({ "token": token, "user": user_json(&s.user, false) })))
}

async fn logout(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        server.store.delete_session(&token_hash(token.trim())).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn me(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult {
    let s = staff(&server, &headers).await?;
    Ok(Json(user_json(&s.user, false)))
}

async fn overview(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult {
    staff(&server, &headers).await?;
    let o = server.store.overview(now()).await?;
    let live = server.with(|l| l.live());
    Ok(Json(json!({
        "stored": o,
        "online": live.players.len(),
        "addresses": server.with(|l| l.distinct_addresses()),
        "matches_live": live.rooms.iter().filter(|r| !r.finished).count(),
        "tables_open": live.tables,
        "tournaments": live.tournaments.len(),
        "uptime_secs": server.started.elapsed().as_secs(),
        "version": env!("CARGO_PKG_VERSION"),
        "persistent": server.store.is_persistent(),
        "motd": server.with(|l| l.rules.motd.clone()),
    })))
}

async fn live(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult {
    staff(&server, &headers).await?;
    let live = server.with(|l| l.live());
    Ok(Json(serde_json::to_value(live).unwrap_or_default()))
}

#[derive(Deserialize)]
struct UsersQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    page: u32,
}

async fn users(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Query(q): Query<UsersQuery>,
) -> ApiResult {
    staff(&server, &headers).await?;
    let per_page = 50;
    let (list, total) =
        server.store.search_users(q.q.trim(), q.page.min(10_000) * per_page, per_page).await?;
    let rows: Vec<Value> =
        server.with(|l| list.iter().map(|u| user_json(u, l.is_online(&u.username))).collect());
    Ok(Json(json!({ "users": rows, "total": total, "page": q.page, "per_page": per_page })))
}

async fn find(server: &Server, name: &str) -> Result<User, ApiError> {
    server
        .store
        .user_by_name(name)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "No such user."))
}

async fn user_detail(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult {
    staff(&server, &headers).await?;
    let u = find(&server, &name).await?;
    let (career, recovery) =
        tokio::join!(server.store.career(u.id), server.store.payment_email(u.id));
    let (history, reports, achievements, wallet, ledger, payments) = tokio::join!(
        server.store.history(u.id, 50),
        server.store.reports_about(u.id, 50),
        server.store.achievements(u.id),
        server.store.wallet(u.id),
        server.store.ledger(u.id, 50),
        server.store.payments_of(u.id, 50),
    );
    let online = server.with(|l| l.is_online(&u.username));
    let history: Vec<Value> = history?
        .into_iter()
        .map(|h| {
            json!({
                "opponent": h.opponent, "won": h.won, "reason": h.reason, "rated": h.rated,
                "delta": h.delta, "rating_after": h.rating_after, "at": h.at,
                "match_id": h.match_id, "replay": h.replay,
            })
        })
        .collect();
    Ok(Json(json!({
        "user": user_json(&u, online),
        "history": history,
        "reports": reports?,
        "achievements": achievements?,
        "wallet": wallet?,
        "ledger": ledger?,
        "payments": payments?,
        "career": career?,
        "payment_email": recovery?,
    })))
}

#[derive(Deserialize, Default)]
struct ActionBody {
    #[serde(default)]
    hours: Option<u32>,
    #[serde(default)]
    minutes: Option<u32>,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    role: Option<Role>,
    #[serde(default)]
    password: String,
    /// Coins to add (negative: remove), for "coins".
    #[serde(default)]
    coins: Option<i64>,
    /// Item code, for "item"; days, for "vip".
    #[serde(default)]
    item: String,
    #[serde(default)]
    days: Option<u32>,
}

async fn user_action(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path((name, action)): Path<(String, String)>,
    body: Option<Json<ActionBody>>,
) -> ApiResult {
    let s = staff(&server, &headers).await?;
    let body = body.map(|b| b.0).unwrap_or_default();
    let target = find(&server, &name).await?;
    // Moderators can't act on other staff; nobody acts on themselves.
    if target.role.is_staff() && s.user.role != Role::Admin {
        return Err(err(StatusCode::FORBIDDEN, "Only administrators can act on staff."));
    }
    if target.id == s.user.id && action != "logout-all" {
        return Err(err(StatusCode::BAD_REQUEST, "You can't do that to yourself."));
    }
    let reason = clean_text(&body.reason, 300);
    let t = now();
    match action.as_str() {
        "ban" => {
            let until = match body.hours {
                Some(h) if h > 0 => t + h as i64 * 3600,
                _ => i64::MAX,
            };
            server.store.set_ban(target.id, Some(until), Some(&reason)).await?;
            server.store.delete_user_sessions(target.id).await?;
            let msg = match body.hours {
                Some(h) if h > 0 => format!("Your account was suspended for {h} hours: {reason}"),
                _ => format!("Your account was banned: {reason}"),
            };
            server.with(|l| l.kick(&target.username, &msg));
            let dur = body.hours.map(|h| format!("{h}h")).unwrap_or_else(|| "permanent".into());
            log(&server, &s, "ban", &target.username, &format!("{dur}: {reason}")).await;
        }
        "unban" => {
            server.store.set_ban(target.id, None, None).await?;
            log(&server, &s, "unban", &target.username, &reason).await;
        }
        "mute" => {
            let minutes = body.minutes.unwrap_or(60).clamp(1, 60 * 24 * 30);
            let until = t + minutes as i64 * 60;
            server.store.set_mute(target.id, Some(until)).await?;
            refresh(&server, &target).await;
            server.with(|l| {
                l.notify(
                    &target.username,
                    snook_proto::ServerMsg::Notice {
                        text: format!("You were muted for {minutes} minutes: {reason}"),
                    },
                )
            });
            log(&server, &s, "mute", &target.username, &format!("{minutes} min: {reason}")).await;
        }
        "unmute" => {
            server.store.set_mute(target.id, None).await?;
            refresh(&server, &target).await;
            log(&server, &s, "unmute", &target.username, &reason).await;
        }
        "kick" => {
            let msg = if reason.is_empty() {
                "You were disconnected by a moderator.".to_string()
            } else {
                format!("You were disconnected by a moderator: {reason}")
            };
            let was = server.with(|l| l.kick(&target.username, &msg));
            if !was {
                return Err(err(StatusCode::BAD_REQUEST, "That player is not online."));
            }
            log(&server, &s, "kick", &target.username, &reason).await;
        }
        "logout-all" => {
            server.store.delete_user_sessions(target.id).await?;
            server.with(|l| l.kick(&target.username, "Please sign in again."));
            log(&server, &s, "logout-all", &target.username, "").await;
        }
        "role" => {
            require_admin(&s)?;
            let role = body.role.ok_or_else(|| err(StatusCode::BAD_REQUEST, "Missing role."))?;
            server.store.set_role(target.id, role).await?;
            server.store.delete_user_sessions(target.id).await?;
            refresh(&server, &target).await;
            log(&server, &s, "role", &target.username, &format!("{role:?}")).await;
        }
        "password" => {
            require_admin(&s)?;
            if !valid_password(&body.password) {
                return Err(err(StatusCode::BAD_REQUEST, "Passwords need at least 8 characters."));
            }
            let hash = server
                .hash_password(&body.password)
                .await
                .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "Could not hash."))?;
            server.store.set_password(target.id, &hash).await?;
            server.store.delete_user_sessions(target.id).await?;
            server.with(|l| l.kick(&target.username, "Your password was changed. Sign in again."));
            log(&server, &s, "password-reset", &target.username, "").await;
        }
        // Shop support (refund goodwill, contest prizes…): administrators only,
        // always with a reason, always in the audit log.
        "coins" => {
            require_admin(&s)?;
            let coins = body.coins.filter(|c| *c != 0 && c.abs() <= 1_000_000);
            let coins = coins.ok_or_else(|| err(StatusCode::BAD_REQUEST, "Give a coin amount."))?;
            if reason.is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, "Give a reason."));
            }
            server.store.credit(target.id, coins, &format!("staff: {reason}"), None, t).await?;
            crate::shop::push_wallet(&server, &target).await;
            log(&server, &s, "coins", &target.username, &format!("{coins:+}: {reason}")).await;
        }
        "item" => {
            require_admin(&s)?;
            if snook_proto::shop::item(&body.item).is_none() {
                return Err(err(StatusCode::BAD_REQUEST, "No such item."));
            }
            server.store.grant_item(target.id, &body.item, "staff", t).await?;
            crate::shop::push_wallet(&server, &target).await;
            log(&server, &s, "item", &target.username, &format!("{}: {reason}", body.item)).await;
        }
        "vip" => {
            require_admin(&s)?;
            let days = body.days.filter(|d| *d > 0 && *d <= 366);
            let days = days.ok_or_else(|| err(StatusCode::BAD_REQUEST, "Give 1–366 days."))?;
            let w = server.store.wallet(target.id).await?;
            let until = w.vip_until.max(t) + days as i64 * 86_400;
            server.store.set_vip(target.id, until, false).await?;
            crate::shop::push_wallet(&server, &target).await;
            log(&server, &s, "vip", &target.username, &format!("+{days} days: {reason}")).await;
        }
        _ => return Err(err(StatusCode::NOT_FOUND, "Unknown action.")),
    }
    let fresh = find(&server, &target.username).await?;
    let online = server.with(|l| l.is_online(&fresh.username));
    Ok(Json(json!({ "ok": true, "user": user_json(&fresh, online) })))
}

/// Reloads an online player's account into the lounge.
async fn refresh(server: &Server, target: &User) {
    if let Ok(Some(fresh)) = server.store.user_by_id(target.id).await {
        server.with(|l| l.refresh_user(fresh));
    }
}

#[derive(Deserialize)]
struct ReportsQuery {
    #[serde(default)]
    status: String,
}

async fn reports(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Query(q): Query<ReportsQuery>,
) -> ApiResult {
    staff(&server, &headers).await?;
    let status = match q.status.as_str() {
        "all" => None,
        "actioned" => Some(1),
        "dismissed" => Some(2),
        _ => Some(0),
    };
    Ok(Json(json!({ "reports": server.store.reports(status, 200).await? })))
}

#[derive(Deserialize)]
struct ResolveBody {
    status: String,
    #[serde(default)]
    note: String,
}

async fn resolve_report(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<ResolveBody>,
) -> ApiResult {
    let s = staff(&server, &headers).await?;
    let status = match body.status.as_str() {
        "actioned" => 1,
        "dismissed" => 2,
        "open" => 0,
        _ => return Err(err(StatusCode::BAD_REQUEST, "Unknown status.")),
    };
    let note = clean_text(&body.note, 300);
    if !server.store.resolve_report(id, s.user.id, status, &note, now()).await? {
        return Err(err(StatusCode::NOT_FOUND, "No such report."));
    }
    log(&server, &s, "report", &format!("#{id}"), &format!("{}: {note}", body.status)).await;
    Ok(Json(json!({ "ok": true })))
}

async fn audit(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult {
    staff(&server, &headers).await?;
    Ok(Json(json!({ "entries": server.store.audit_log(300).await? })))
}

#[derive(Deserialize)]
struct TextBody {
    text: String,
}

async fn announce(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(body): Json<TextBody>,
) -> ApiResult {
    let s = staff(&server, &headers).await?;
    let text = clean_text(&body.text, snook_proto::MAX_CHAT_LEN);
    if text.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Write a message."));
    }
    server.with(|l| l.announce_text(&text));
    log(&server, &s, "announce", "lounge", &text).await;
    Ok(Json(json!({ "ok": true })))
}

async fn motd(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(body): Json<TextBody>,
) -> ApiResult {
    let s = staff(&server, &headers).await?;
    require_admin(&s)?;
    server.with(|l| l.set_motd(&body.text));
    log(&server, &s, "motd", "lounge", &clean_text(&body.text, 300)).await;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::flag_outliers;
    use crate::store::AimRow;

    #[test]
    fn only_far_outliers_are_flagged() {
        // A tier of honest players around 20–35% on hard shots, and one at 80%.
        let mut rows: Vec<AimRow> = (0..60)
            .map(|k| AimRow {
                user_id: k,
                username: format!("p{k}"),
                rating: 1550,
                attempts: 100,
                made: 20 + (k as u32 % 16),
            })
            .collect();
        rows.push(AimRow {
            user_id: 99,
            username: "aimbot".into(),
            rating: 1560,
            attempts: 120,
            made: 96,
        });
        // A strong player in a higher tier is compared with that tier, not this one.
        rows.push(AimRow {
            user_id: 100,
            username: "master".into(),
            rating: 1750,
            attempts: 80,
            made: 30,
        });
        let flagged = flag_outliers(&rows);
        assert_eq!(flagged.len(), 1, "{flagged:?}");
        assert_eq!(flagged[0]["username"], "aimbot");
    }
}
