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
use snookered_proto::shop::{
    self, COIN_PACKS, Category, Price, VIP_GIFT, VIP_GIFT_SECS, VIP_MONTHLY_COINS, VIP_PLAN,
    WELCOME_COINS, WELCOME_ITEMS, WELCOME_PACK,
};
use snookered_proto::{ClientMsg, PackOffer, ServerMsg};

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

/// What others see of a player: the VIP crown and the items in use.
pub fn look_of(w: &Wallet, achievements: &[String], t: i64) -> Look {
    let item = |c: Category| effective(w, achievements, c, t).to_string();
    Look {
        vip: is_vip(w, t),
        cue: item(Category::Cue),
        chalk: item(Category::Chalk),
        cloth: item(Category::Cloth),
        balls: item(Category::Balls),
        frame: item(Category::Frame),
        name_style: item(Category::Name),
        effect: item(Category::Effect),
    }
}

async fn achievements_of(server: &Server, id: i64) -> Vec<String> {
    server.store.achievements(id).await.unwrap_or_default().into_iter().map(|(c, _)| c).collect()
}

/// Sends `user` their wallet and updates what others see of them.
pub async fn push_wallet(server: &Server, user: &User) {
    let (w, achievements) =
        tokio::join!(server.store.wallet(user.id), achievements_of(server, user.id));
    let Ok(w) = w else { return };
    let look = look_of(&w, &achievements, now());
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
            vip_gift_price: stripe.label(VIP_GIFT).unwrap_or_default(),
            welcome_price: stripe.label(WELCOME_PACK).unwrap_or_default(),
        },
        None => ServerMsg::ShopInfo {
            payments: false,
            packs: Vec::new(),
            vip_price: String::new(),
            vip_coins: VIP_MONTHLY_COINS,
            vip_gift_price: String::new(),
            welcome_price: String::new(),
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
                    send(ServerMsg::ItemBought { code: item.code.to_string(), gift_to: None });
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
        ClientMsg::GiftItem { to, code } => {
            let Some(item) = shop::item(&code) else { return error("No such item.") };
            let Some(price) = item.coins() else {
                return error("Only items sold for coins can be given.");
            };
            let friend = match friend_named(server, me, &to).await {
                Ok(f) => f,
                Err(e) => return error(e),
            };
            match server.store.gift_item(me.id, friend.id, item.code, price as i64, now()).await {
                Ok(BuyOutcome::Bought) => {
                    tracing::info!(from = me.id, to = friend.id, item = item.code, "item given");
                    send(ServerMsg::ItemBought {
                        code: item.code.to_string(),
                        gift_to: Some(friend.username.clone()),
                    });
                    push_wallet(server, me).await;
                    let msg = ServerMsg::GiftReceived {
                        from: me.username.clone(),
                        code: item.code.to_string(),
                    };
                    server.with(|l| l.notify(&friend.username, msg));
                    push_wallet(server, &friend).await;
                }
                Ok(BuyOutcome::AlreadyOwned) => error("Your friend already has this item."),
                Ok(BuyOutcome::NotEnoughCoins) => error("Not enough coins."),
                Err(e) => {
                    tracing::error!("gift {code} from {}: {e}", me.username);
                    error("The purchase failed; no coins were taken.")
                }
            }
        }
        ClientMsg::Equip { code } => {
            let Some(item) = shop::item(&code) else { return error("No such item.") };
            let (w, achievements) =
                tokio::join!(server.store.wallet(me.id), achievements_of(server, me.id));
            let w = w.unwrap_or_default();
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
        ClientMsg::BuyWelcomePack => {
            let Some(stripe) = &server.stripe else { return error("Purchases are not available.") };
            let w = server.store.wallet(me.id).await.unwrap_or_default();
            if shop::has_welcome_pack(&w.owned) {
                return error("You already have the welcome pack.");
            }
            match stripe
                .checkout_pack(me.id, &me.username, w.stripe_customer.as_deref(), WELCOME_PACK)
                .await
            {
                Ok(url) => send(ServerMsg::PaymentPage { url }),
                Err(e) => {
                    tracing::warn!("welcome pack checkout for {}: {e}", me.username);
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
        ClientMsg::GiftVip { to } => {
            let Some(stripe) = &server.stripe else { return error("Purchases are not available.") };
            let friend = match friend_named(server, me, &to).await {
                Ok(f) => f,
                Err(e) => return error(e),
            };
            // A subscriber's months come from the subscription; a gift on top
            // would be cut short by its next renewal.
            let theirs = server.store.wallet(friend.id).await.unwrap_or_default();
            if is_vip(&theirs, now()) {
                return error("Your friend is already VIP.");
            }
            let customer = server.store.wallet(me.id).await.ok().and_then(|w| w.stripe_customer);
            match stripe
                .checkout_vip_gift(me.id, &me.username, customer.as_deref(), friend.id)
                .await
            {
                Ok(url) => send(ServerMsg::PaymentPage { url }),
                Err(e) => {
                    tracing::warn!("VIP gift checkout for {}: {e}", me.username);
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

/// The friend called `name`, or why a present can't go to them.
async fn friend_named(server: &Server, me: &User, name: &str) -> Result<User, &'static str> {
    let Ok(Some(friend)) = server.store.user_by_name(name.trim()).await else {
        return Err("No such player.");
    };
    let friends = server.store.friends(me.id).await.unwrap_or_default();
    let is_friend = friends
        .iter()
        .any(|(u, st)| u.id == friend.id && *st == crate::store::FriendState::Friends);
    if !is_friend {
        return Err("You can only give items to your friends.");
    }
    Ok(friend)
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
<title>Snookered</title><body style='background:#140d08;color:#efe3cc;font:18px system-ui;text-align:center;padding:12vh 20px'>\
<h1 style='color:#e8bf61'>Obrigado! · Thank you!</h1><p>Pode voltar ao jogo: a compra aparece em instantes.</p>\
<p>You can go back to the game: your purchase appears in a moment.</p></body>";

const PAGE_CANCELLED: &str = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>\
<title>Snookered</title><body style='background:#140d08;color:#efe3cc;font:18px system-ui;text-align:center;padding:12vh 20px'>\
<h1 style='color:#e8bf61'>Compra cancelada · Purchase cancelled</h1><p>Nada foi cobrado. · Nothing was charged.</p></body>";

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// The e-mail the customer gave on the payment page (a checkout session
/// or an invoice).
fn email_of(obj: &Value) -> Option<String> {
    obj.pointer("/customer_details/email")
        .and_then(Value::as_str)
        .or_else(|| str_of(obj, "customer_email"))
        .map(|e| e.trim().to_lowercase())
        .filter(|e| crate::account::valid_email(e))
}

/// After a payment: a player without a recovery e-mail is offered the one
/// they just paid with.
async fn offer_email(server: &Server, user: &User) {
    if user.recovery_email.is_some() {
        return;
    }
    if let Ok(Some(email)) = server.store.payment_email(user.id).await {
        let msg = ServerMsg::Account { recovery_email: None, payment_email: Some(email) };
        server.with(|l| l.notify(&user.username, msg));
    }
}

async fn user_of(server: &Server, obj: &Value) -> Option<User> {
    // Our own reference first, then the customer.
    let id = str_of(obj, "client_reference_id")
        .or_else(|| obj.get("metadata").and_then(|m| str_of(m, "user_id")))
        // A subscription's invoice: where the subscription's own metadata is
        // copied to (the place moved between Stripe API versions).
        .or_else(|| {
            [
                "/subscription_details/metadata/user_id",
                "/parent/subscription_details/metadata/user_id",
                "/lines/data/0/metadata/user_id",
            ]
            .iter()
            .find_map(|p| obj.pointer(p).and_then(Value::as_str))
        })
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

/// A VIP month was paid with `invoice`: VIP until `end` and the month's
/// coins. Safe to run more than once for the same invoice.
async fn vip_paid(
    server: &Server,
    user: &User,
    invoice: &str,
    end: i64,
    amount: i64,
    currency: &str,
    email: Option<String>,
) -> Result<(), String> {
    let err = |e: crate::store::StoreError| e.to_string();
    // A month that was refunded stays refunded, whatever arrives after.
    if server.store.payment(invoice).await.map_err(err)?.is_some_and(|p| p.refunded) {
        return Ok(());
    }
    server.store.set_vip(user.id, end + VIP_GRACE, false).await.map_err(err)?;
    let first = server
        .store
        .record_payment(&Payment {
            id: invoice.to_string(),
            user_id: user.id,
            kind: "vip".into(),
            // What a refund takes back: only coins that were given.
            coins: if amount > 0 { VIP_MONTHLY_COINS as i64 } else { 0 },
            amount,
            currency: currency.to_string(),
            at: now(),
            refunded: false,
            email,
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
    if first {
        tracing::info!(user = user.id, until = end, "VIP paid");
        notify(server, user, "Welcome to VIP!").await;
        offer_email(server, user).await;
    } else {
        push_wallet(server, user).await;
    }
    Ok(())
}

/// A month of VIP given by `giver` to the player with id `friend` was paid
/// (checkout session `obj`). Safe to run more than once.
async fn vip_gift_paid(
    server: &Server,
    giver: &User,
    friend: &str,
    obj: &Value,
) -> Result<(), String> {
    let err = |e: crate::store::StoreError| e.to_string();
    let Some(friend) = friend.parse::<i64>().ok() else {
        return Err(format!("bad gift_vip_to {friend}"));
    };
    let Some(friend) = server.store.user_by_id(friend).await.map_err(err)? else {
        tracing::warn!("VIP gift for a deleted player");
        return Ok(());
    };
    let payment_id = str_of(obj, "payment_intent").or(str_of(obj, "id")).unwrap_or("");
    // A month that was refunded stays refunded, whatever arrives after.
    if server.store.payment(payment_id).await.map_err(err)?.is_some_and(|p| p.refunded) {
        return Ok(());
    }
    // Recorded under the friend: a refund takes back what they were given.
    let first = server
        .store
        .record_payment(&Payment {
            id: payment_id.to_string(),
            user_id: friend.id,
            kind: "vip_gift".into(),
            coins: VIP_MONTHLY_COINS as i64,
            amount: obj["amount_total"].as_i64().unwrap_or(0),
            currency: str_of(obj, "currency").unwrap_or("").to_string(),
            at: now(),
            refunded: false,
            email: email_of(obj),
        })
        .await
        .map_err(err)?;
    // The month counts from the payment. Two gifts at once add up; a
    // repeated delivery only makes sure the month is there.
    let from = obj["created"].as_i64().unwrap_or(now());
    let until = if first {
        let w = server.store.wallet(friend.id).await.map_err(err)?;
        w.vip_until.max(from) + VIP_GIFT_SECS
    } else {
        from + VIP_GIFT_SECS
    };
    server.store.set_vip(friend.id, until, false).await.map_err(err)?;
    server
        .store
        .credit(
            friend.id,
            VIP_MONTHLY_COINS as i64,
            &format!("vip gift from {}", giver.username),
            Some(&format!("vipgift:{payment_id}")),
            now(),
        )
        .await
        .map_err(err)?;
    if first {
        tracing::info!(from = giver.id, to = friend.id, "VIP given");
        let msg = ServerMsg::VipGift { from: giver.username.clone(), to: friend.username.clone() };
        server.with(|l| {
            l.notify(&friend.username, msg.clone());
            l.notify(&giver.username, msg);
        });
        offer_email(server, giver).await;
    }
    push_wallet(server, &friend).await;
    Ok(())
}

/// The welcome pack was paid (checkout session `obj`): its items and
/// coins. Safe to run more than once.
async fn welcome_paid(server: &Server, user: &User, obj: &Value) -> Result<(), String> {
    let err = |e: crate::store::StoreError| e.to_string();
    let payment_id = str_of(obj, "payment_intent").or(str_of(obj, "id")).unwrap_or("");
    if server.store.payment(payment_id).await.map_err(err)?.is_some_and(|p| p.refunded) {
        return Ok(());
    }
    server
        .store
        .record_payment(&Payment {
            id: payment_id.to_string(),
            user_id: user.id,
            kind: "welcome".into(),
            coins: WELCOME_COINS as i64,
            amount: obj["amount_total"].as_i64().unwrap_or(0),
            currency: str_of(obj, "currency").unwrap_or("").to_string(),
            at: now(),
            refunded: false,
            email: email_of(obj),
        })
        .await
        .map_err(err)?;
    for code in WELCOME_ITEMS {
        server.store.grant_item(user.id, code, "welcome pack", now()).await.map_err(err)?;
    }
    let applied = server
        .store
        .credit(
            user.id,
            WELCOME_COINS as i64,
            "welcome pack",
            Some(&format!("pay:{payment_id}")),
            now(),
        )
        .await
        .map_err(err)?;
    if applied {
        tracing::info!(user = user.id, "welcome pack bought");
        // Bought to be used: the new cue goes on straight away.
        let cue = WELCOME_ITEMS[0];
        let _ = server.store.equip(user.id, Category::Cue.code(), cue).await;
        notify(server, user, "Your welcome pack is here!").await;
        offer_email(server, user).await;
    } else {
        push_wallet(server, user).await;
    }
    Ok(())
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
            // A new subscription, paid: its first month. The invoice says the
            // same (and the exact dates), but the two can come in any order,
            // and the invoice alone doesn't always say who the player is.
            if str_of(obj, "mode") == Some("subscription")
                && str_of(obj, "payment_status") == Some("paid")
                && let Some(invoice) = str_of(obj, "invoice")
            {
                let amount = obj["amount_total"].as_i64().unwrap_or(0);
                let currency = str_of(obj, "currency").unwrap_or("");
                let email = email_of(obj);
                vip_paid(server, &user, invoice, now() + 31 * 86_400, amount, currency, email)
                    .await?;
            }
            // Coin packs; Pix and boleto are paid later (async_payment_succeeded).
            if str_of(obj, "mode") == Some("payment")
                && str_of(obj, "payment_status") == Some("paid")
                && let Some(friend) = obj.pointer("/metadata/gift_vip_to").and_then(Value::as_str)
            {
                return vip_gift_paid(server, &user, friend, obj).await;
            }
            if str_of(obj, "mode") == Some("payment")
                && str_of(obj, "payment_status") == Some("paid")
            {
                let pack = obj.pointer("/metadata/pack").and_then(Value::as_str).unwrap_or("");
                if pack == WELCOME_PACK {
                    return welcome_paid(server, &user, obj).await;
                }
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
                        email: email_of(obj),
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
                    offer_email(server, &user).await;
                }
            }
        }
        "invoice.paid" => {
            // An invoice that arrives before the checkout that says whose
            // customer this is: the checkout event grants the month (above).
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
            let amount = obj["amount_paid"].as_i64().unwrap_or(0);
            let currency = str_of(obj, "currency").unwrap_or("");
            vip_paid(server, &user, invoice, end, amount, currency, email_of(obj)).await?;
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
            // A partial refund is a courtesy from the staff: the purchase stands.
            if kind == "charge.refunded" && obj["refunded"].as_bool() == Some(false) {
                return Ok(());
            }
            // Coin packs are recorded under the payment, VIP months under the
            // invoice; a charge carries both ids.
            let mut found = None;
            for id in [str_of(obj, "payment_intent"), str_of(obj, "invoice")].into_iter().flatten()
            {
                if let Some(p) = server.store.payment(id).await.map_err(err)? {
                    found = Some(p);
                    break;
                }
            }
            let Some(p) = found else { return Ok(()) };
            let first = server.store.refund_payment(&p.id).await.map_err(err)?;
            // Keyed by the payment: taken back exactly once, even when an
            // earlier delivery stopped half-way.
            let taken = server
                .store
                .credit(p.user_id, -p.coins, kind, Some(&format!("refund:{}", p.id)), now())
                .await
                .map_err(err)?;
            if first && (p.kind == "vip" || p.kind == "vip_gift") {
                server.store.set_vip(p.user_id, now(), true).await.map_err(err)?;
            }
            if first || taken {
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
