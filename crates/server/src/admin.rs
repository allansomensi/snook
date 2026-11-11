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
    })
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
