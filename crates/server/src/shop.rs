//! The shop on the server: wallets, buying and using items, coin packs and
//! VIP paid through Stripe (see [`crate::payments`]).
//!
//! The server is the only authority: prices, ownership and balances are
//! checked here, never trusted from the game.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde_json::Value;
use snook_proto::shop::{self, COIN_PACKS, Category, Price, VIP_MONTHLY_COINS, VIP_PLAN};
use snook_proto::{ClientMsg, PackOffer, ServerMsg};

use crate::lounge::Look;
use crate::store::{BuyOutcome, Payment, User, Wallet};
use crate::{Server, now};

/// A few hours of VIP beyond the paid period, so a renewal that arrives a
/// little late never interrupts it.
const VIP_GRACE: i64 = 6 * 3600;

pub fn is_vip(w: &Wallet, now: i64) -> bool {
    w.vip_until > now
}

/// The item actually used in `category` (what is equipped, if still usable;
/// else the default).
pub fn effective(
    w: &Wallet,
    achievements: &[String],
    category: Category,
    now: i64,
) -> &'static str {
    w.equipped
        .get(category.code())
        .and_then(|code| shop::item(code))
        .filter(|i| {
            i.category == category && shop::can_use(i, &w.owned, achievements, is_vip(w, now))
        })
        .map(|i| i.code)
        .unwrap_or_else(|| category.default_item())
}

fn wallet_msg(w: &Wallet) -> ServerMsg {
    ServerMsg::Wallet {
        coins: w.coins,
        vip_until: w.vip_until,
        owned: w.owned.clone(),
        equipped: w.equipped.iter().map(|(c, i)| (c.clone(), i.clone())).collect(),
    }
}

/// Sends `user` their wallet and updates what others see of them.
pub async fn push_wallet(server: &Server, user: &User) {
    let Ok(w) = server.store.wallet(user.id).await else { return };
    let achievements: Vec<String> = server
        .store
        .achievements(user.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    let t = now();
    let look = Look {
        vip: is_vip(&w, t),
        cue: effective(&w, &achievements, Category::Cue, t).to_string(),
        chalk: effective(&w, &achievements, Category::Chalk, t).to_string(),
    };
    let msg = wallet_msg(&w);
    server.with(|l| {
        l.notify(&user.username, msg);
        l.set_look(&user.username, look);
    });
}

fn shop_info(server: &Server) -> ServerMsg {
    match &server.stripe {
        Some(stripe) => ServerMsg::ShopInfo {
            payments: true,
            packs: COIN_PACKS
                .iter()
                .filter_map(|p| {
                    Some(PackOffer {
                        code: p.code.to_string(),
                        coins: p.coins,
                        bonus: p.bonus,
                        price: stripe.label(p.code)?,
                    })
                })
                .collect(),
            vip_price: stripe.label(VIP_PLAN).unwrap_or_default(),
            vip_coins: VIP_MONTHLY_COINS,
        },
        None => ServerMsg::ShopInfo {
            payments: false,
            packs: Vec::new(),
            vip_price: String::new(),
            vip_coins: VIP_MONTHLY_COINS,
        },
    }
}

/// A shop message from a signed-in player (never a guest).
pub async fn handle(server: &Arc<Server>, me: &User, msg: ClientMsg, send: &impl Fn(ServerMsg)) {
    let error = |text: &str| send(ServerMsg::Error { message: text.to_string() });
    match msg {
        ClientMsg::Shop => {
            send(shop_info(server));
            push_wallet(server, me).await;
        }
        ClientMsg::BuyItem { code } => {
            let Some(item) = shop::item(&code) else { return error("No such item.") };
            let Price::Coins(price) = item.price else {
                return error("This item can't be bought with coins.");
            };
            match server.store.buy_item(me.id, item.code, price as i64, now()).await {
                Ok(BuyOutcome::Bought) => {
                    tracing::info!(user = me.id, item = item.code, price, "item bought");
                    // Bought to be used: put it on straight away.
                    let _ = server.store.equip(me.id, item.category.code(), item.code).await;
                    send(ServerMsg::Notice { text: "Purchase complete.".into() });
                    push_wallet(server, me).await;
                }
                Ok(BuyOutcome::AlreadyOwned) => error("You already have this item."),
                Ok(BuyOutcome::NotEnoughCoins) => error("Not enough coins."),
                Err(e) => {
                    tracing::error!("buying {code} for {}: {e}", me.username);
                    error("The purchase failed; no coins were taken.")
                }
            }
        }
        ClientMsg::Equip { code } => {
            let Some(item) = shop::item(&code) else { return error("No such item.") };
            let w = server.store.wallet(me.id).await.unwrap_or_default();
            let achievements: Vec<String> = server
                .store
                .achievements(me.id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(c, _)| c)
                .collect();
            if !shop::can_use(item, &w.owned, &achievements, is_vip(&w, now())) {
                return error("You don't have this item.");
            }
            if server.store.equip(me.id, item.category.code(), item.code).await.is_ok() {
                push_wallet(server, me).await;
            }
        }
        ClientMsg::BuyCoins { pack } => {
            let Some(stripe) = &server.stripe else { return error("Purchases are not available.") };
            if shop::coin_pack(&pack).is_none() {
                return error("This pack is not on sale.");
            }
            let customer = server.store.wallet(me.id).await.ok().and_then(|w| w.stripe_customer);
            match stripe.checkout_pack(me.id, &me.username, customer.as_deref(), &pack).await {
                Ok(url) => send(ServerMsg::PaymentPage { url }),
                Err(e) => {
                    tracing::warn!("checkout for {}: {e}", me.username);
                    error("The payment page could not be opened. Try again in a moment.")
                }
            }
        }
        ClientMsg::SubscribeVip => {
            let Some(stripe) = &server.stripe else { return error("Purchases are not available.") };
            let w = server.store.wallet(me.id).await.unwrap_or_default();
            if is_vip(&w, now()) {
                return error("You are already VIP.");
            }
            match stripe.checkout_vip(me.id, &me.username, w.stripe_customer.as_deref()).await {
                Ok(url) => send(ServerMsg::PaymentPage { url }),
                Err(e) => {
                    tracing::warn!("VIP checkout for {}: {e}", me.username);
                    error("The payment page could not be opened. Try again in a moment.")
                }
            }
        }
        ClientMsg::ManageVip => {
            let Some(stripe) = &server.stripe else { return error("Purchases are not available.") };
            let w = server.store.wallet(me.id).await.unwrap_or_default();
            let Some(customer) = w.stripe_customer else {
                return error("There is no subscription to manage.");
            };
            match stripe.portal(&customer).await {
                Ok(url) => send(ServerMsg::PaymentPage { url }),
                Err(e) => {
                    tracing::warn!("portal for {}: {e}", me.username);
                    error("The page could not be opened. Try again in a moment.")
                }
            }
        }
        _ => {}
    }
}

/// `POST /stripe/webhook`: Stripe's signed notifications.
pub async fn webhook(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(stripe) = &server.stripe else { return StatusCode::NOT_FOUND.into_response() };
    let signature = headers.get("stripe-signature").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !stripe.verify(&body, signature, now()) {
        tracing::warn!("webhook with a bad signature");
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(event) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match apply_event(&server, &event).await {
        Ok(()) => StatusCode::OK.into_response(),
        // Stripe retries: every step is idempotent.
        Err(e) => {
            tracing::error!("webhook {}: {e}", event["type"]);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The pages Stripe sends the player back to.
pub async fn thanks() -> Html<&'static str> {
    Html(PAGE_THANKS)
}

pub async fn cancelled() -> Html<&'static str> {
    Html(PAGE_CANCELLED)
}

const PAGE_THANKS: &str = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>\
<title>Snook</title><body style='background:#140d08;color:#efe3cc;font:18px system-ui;text-align:center;padding:12vh 20px'>\
<h1 style='color:#e8bf61'>Obrigado! · Thank you!</h1><p>Pode voltar ao jogo: a compra aparece em instantes.</p>\
<p>You can go back to the game: your purchase appears in a moment.</p></body>";

const PAGE_CANCELLED: &str = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>\
<title>Snook</title><body style='background:#140d08;color:#efe3cc;font:18px system-ui;text-align:center;padding:12vh 20px'>\
<h1 style='color:#e8bf61'>Compra cancelada · Purchase cancelled</h1><p>Nada foi cobrado. · Nothing was charged.</p></body>";

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

async fn user_of(server: &Server, obj: &Value) -> Option<User> {
    // Our own reference first, then the customer.
    let id = str_of(obj, "client_reference_id")
        .or_else(|| obj.get("metadata").and_then(|m| str_of(m, "user_id")))
        .or_else(|| obj.pointer("/subscription_details/metadata/user_id").and_then(Value::as_str))
        .and_then(|s| s.parse::<i64>().ok());
    let id = match id {
        Some(id) => Some(id),
        None => match str_of(obj, "customer") {
            Some(c) => server.store.user_by_stripe_customer(c).await.ok().flatten(),
            None => None,
        },
    }?;
    server.store.user_by_id(id).await.ok().flatten()
}

async fn notify(server: &Server, user: &User, text: &str) {
    let text = text.to_string();
    server.with(|l| l.notify(&user.username, ServerMsg::Notice { text }));
    push_wallet(server, user).await;
}

/// Applies one Stripe event. Safe to run more than once.
pub async fn apply_event(server: &Server, event: &Value) -> Result<(), String> {
    let kind = str_of(event, "type").unwrap_or("");
    let obj = event.pointer("/data/object").ok_or("no object")?;
    let err = |e: crate::store::StoreError| e.to_string();
    match kind {
        "checkout.session.completed" | "checkout.session.async_payment_succeeded" => {
            let Some(user) = user_of(server, obj).await else {
                tracing::warn!("{kind} for an unknown player");
                return Ok(());
            };
            if let Some(c) = str_of(obj, "customer") {
                server.store.set_stripe_customer(user.id, c).await.map_err(err)?;
            }
            // Coin packs; Pix and boleto are paid later (async_payment_succeeded).
            if str_of(obj, "mode") == Some("payment")
                && str_of(obj, "payment_status") == Some("paid")
            {
                let pack = obj.pointer("/metadata/pack").and_then(Value::as_str).unwrap_or("");
                let Some(pack) = shop::coin_pack(pack) else {
                    return Err(format!("unknown pack {pack}"));
                };
                let coins = (pack.coins + pack.bonus) as i64;
                let payment_id = str_of(obj, "payment_intent").or(str_of(obj, "id")).unwrap_or("");
                server
                    .store
                    .record_payment(&Payment {
                        id: payment_id.to_string(),
                        user_id: user.id,
                        kind: "coins".into(),
                        coins,
                        amount: obj["amount_total"].as_i64().unwrap_or(0),
                        currency: str_of(obj, "currency").unwrap_or("").to_string(),
                        at: now(),
                        refunded: false,
                    })
                    .await
                    .map_err(err)?;
                let applied = server
                    .store
                    .credit(
                        user.id,
                        coins,
                        &format!("pack {}", pack.code),
                        Some(&format!("pay:{payment_id}")),
                        now(),
                    )
                    .await
                    .map_err(err)?;
                if applied {
                    tracing::info!(user = user.id, coins, "coins bought");
                    notify(server, &user, "Coins added to your wallet.").await;
                }
            }
        }
        "invoice.paid" => {
            let Some(user) = user_of(server, obj).await else {
                tracing::warn!("invoice.paid for an unknown player");
                return Ok(());
            };
            let invoice = str_of(obj, "id").unwrap_or("");
            let end = obj
                .pointer("/lines/data/0/period/end")
                .and_then(Value::as_i64)
                .or_else(|| obj["period_end"].as_i64())
                .unwrap_or(now() + 31 * 86_400);
            server.store.set_vip(user.id, end + VIP_GRACE, false).await.map_err(err)?;
            let amount = obj["amount_paid"].as_i64().unwrap_or(0);
            server
                .store
                .record_payment(&Payment {
                    id: invoice.to_string(),
                    user_id: user.id,
                    kind: "vip".into(),
                    coins: VIP_MONTHLY_COINS as i64,
                    amount,
                    currency: str_of(obj, "currency").unwrap_or("").to_string(),
                    at: now(),
                    refunded: false,
                })
                .await
                .map_err(err)?;
            // The monthly coins (not for a free trial month).
            if amount > 0 {
                server
                    .store
                    .credit(
                        user.id,
                        VIP_MONTHLY_COINS as i64,
                        "vip month",
                        Some(&format!("vip:{invoice}")),
                        now(),
                    )
                    .await
                    .map_err(err)?;
            }
            tracing::info!(user = user.id, until = end, "VIP paid");
            notify(server, &user, "Welcome to VIP!").await;
        }
        "customer.subscription.deleted" => {
            if let Some(user) = user_of(server, obj).await {
                // The paid period is over (or the subscription was ended now).
                let ended = obj["ended_at"].as_i64().unwrap_or(now());
                let w = server.store.wallet(user.id).await.map_err(err)?;
                if w.vip_until > ended {
                    server.store.set_vip(user.id, ended, true).await.map_err(err)?;
                }
                push_wallet(server, &user).await;
            }
        }
        "charge.refunded" | "charge.dispute.created" => {
            // The money went back: so do the coins (the balance may go below
            // zero; nothing can be bought until it is positive again).
            let payment_id = str_of(obj, "payment_intent")
                .map(str::to_string)
                .or_else(|| str_of(obj, "invoice").map(str::to_string));
            let Some(payment_id) = payment_id else { return Ok(()) };
            let Some(p) = server.store.payment(&payment_id).await.map_err(err)? else {
                return Ok(());
            };
            if server.store.refund_payment(&p.id).await.map_err(err)? {
                server
                    .store
                    .credit(p.user_id, -p.coins, kind, Some(&format!("refund:{}", p.id)), now())
                    .await
                    .map_err(err)?;
                if p.kind == "vip" {
                    server.store.set_vip(p.user_id, now(), true).await.map_err(err)?;
                }
                tracing::warn!(user = p.user_id, payment = %p.id, "payment reversed ({kind})");
                if let Ok(Some(user)) = server.store.user_by_id(p.user_id).await {
                    push_wallet(server, &user).await;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
