//! The shop's payment events (as Stripe sends them) and purchases.

use serde_json::json;
use snook_proto::Role;
use snook_proto::shop::VIP_MONTHLY_COINS;
use snook_server::shop::apply_event;
use snook_server::store::NewUser;
use snook_server::{Config, Server, now};

async fn setup() -> (std::sync::Arc<Server>, i64) {
    let server = Server::new(Config::default()).await.unwrap();
    let user = server
        .store
        .create_user(&NewUser {
            username: "alice",
            password_hash: "h",
            country: "BR",
            rating: 1200,
            role: Role::Player,
            ip: "1.2.3.4",
        })
        .await
        .unwrap()
        .unwrap();
    (server, user.id)
}

fn checkout(user: i64, status: &str, kind: &str) -> serde_json::Value {
    json!({
        "type": kind,
        "data": { "object": {
            "id": "cs_1", "mode": "payment", "payment_status": status,
            "client_reference_id": user.to_string(), "customer": "cus_9",
            "payment_intent": "pi_7", "amount_total": 990, "currency": "brl",
            "metadata": { "pack": "coins_1200", "user_id": user.to_string() }
        }}
    })
}

#[tokio::test]
async fn coin_packs_are_credited_once_and_taken_back_on_refund() {
    let (server, id) = setup().await;
    // Pix: the session completes unpaid, coins arrive when the payment does.
    apply_event(&server, &checkout(id, "unpaid", "checkout.session.completed")).await.unwrap();
    assert_eq!(server.store.wallet(id).await.unwrap().coins, 0);
    let paid = checkout(id, "paid", "checkout.session.async_payment_succeeded");
    apply_event(&server, &paid).await.unwrap();
    apply_event(&server, &paid).await.unwrap(); // delivered twice
    let w = server.store.wallet(id).await.unwrap();
    assert_eq!(w.coins, 1300, "1200 + 100 bonus, once");
    assert_eq!(w.stripe_customer.as_deref(), Some("cus_9"));
    let refund =
        json!({ "type": "charge.refunded", "data": { "object": { "payment_intent": "pi_7" } } });
    apply_event(&server, &refund).await.unwrap();
    apply_event(&server, &refund).await.unwrap();
    assert_eq!(server.store.wallet(id).await.unwrap().coins, 0);
}

#[tokio::test]
async fn vip_invoices_extend_vip_and_bring_coins() {
    let (server, id) = setup().await;
    server.store.set_stripe_customer(id, "cus_9").await.unwrap();
    let end = now() + 30 * 86_400;
    let invoice = json!({
        "type": "invoice.paid",
        "data": { "object": {
            "id": "in_1", "customer": "cus_9", "amount_paid": 1490, "currency": "brl",
            "lines": { "data": [ { "period": { "end": end } } ] }
        }}
    });
    apply_event(&server, &invoice).await.unwrap();
    apply_event(&server, &invoice).await.unwrap();
    let w = server.store.wallet(id).await.unwrap();
    assert!(w.vip_until > end);
    assert_eq!(w.coins, VIP_MONTHLY_COINS as i64);
    // Cancelled now: VIP ends now.
    let deleted = json!({
        "type": "customer.subscription.deleted",
        "data": { "object": { "customer": "cus_9", "ended_at": now() } }
    });
    apply_event(&server, &deleted).await.unwrap();
    assert!(server.store.wallet(id).await.unwrap().vip_until <= now());
}

#[tokio::test]
async fn invoices_in_the_newer_api_find_the_player() {
    let (server, id) = setup().await;
    // No customer linked yet: the player comes from the subscription metadata.
    let invoice = json!({
        "type": "invoice.paid",
        "data": { "object": {
            "id": "in_2", "customer": "cus_new", "amount_paid": 1490, "currency": "brl",
            "parent": { "subscription_details": { "metadata": { "user_id": id.to_string() } } },
            "lines": { "data": [ { "period": { "end": now() + 86_400 } } ] }
        }}
    });
    apply_event(&server, &invoice).await.unwrap();
    assert!(server.store.wallet(id).await.unwrap().vip_until > now());
}

#[tokio::test]
async fn unknown_players_and_events_are_ignored() {
    let (server, _) = setup().await;
    apply_event(&server, &checkout(999, "paid", "checkout.session.completed")).await.unwrap();
    apply_event(&server, &json!({ "type": "ping", "data": { "object": {} } })).await.unwrap();
    assert!(apply_event(&server, &json!({ "type": "x" })).await.is_err(), "no object");
}

/// A refunded VIP month is found by its invoice (the charge names the
/// payment too, which is not how VIP months are recorded).
#[tokio::test]
async fn refunded_vip_months_end_vip_and_take_the_coins_back() {
    let (server, id) = setup().await;
    server.store.set_stripe_customer(id, "cus_9").await.unwrap();
    let invoice = json!({
        "type": "invoice.paid",
        "data": { "object": {
            "id": "in_5", "customer": "cus_9", "amount_paid": 1490, "currency": "brl",
            "lines": { "data": [ { "period": { "end": now() + 30 * 86_400 } } ] }
        }}
    });
    apply_event(&server, &invoice).await.unwrap();
    // Part of the money back: the month stands.
    let partial = json!({
        "type": "charge.refunded",
        "data": { "object": { "payment_intent": "pi_55", "invoice": "in_5", "refunded": false } }
    });
    apply_event(&server, &partial).await.unwrap();
    assert!(server.store.wallet(id).await.unwrap().vip_until > now());
    let refund = json!({
        "type": "charge.refunded",
        "data": { "object": { "payment_intent": "pi_55", "invoice": "in_5", "refunded": true } }
    });
    apply_event(&server, &refund).await.unwrap();
    apply_event(&server, &refund).await.unwrap();
    let w = server.store.wallet(id).await.unwrap();
    assert!(w.vip_until <= now(), "VIP ends with the refund");
    assert_eq!(w.coins, 0, "the month's coins go back, once");
    // The same invoice delivered again later changes nothing.
    apply_event(&server, &invoice).await.unwrap();
    let w = server.store.wallet(id).await.unwrap();
    assert!(w.vip_until <= now());
    assert_eq!(w.coins, 0);
}

/// The first invoice of a subscription can arrive before the checkout that
/// links the customer to the player: the month must not be lost, nor paid twice.
#[tokio::test]
async fn a_new_subscription_is_granted_whatever_the_order_of_events() {
    let (server, id) = setup().await;
    let invoice = json!({
        "type": "invoice.paid",
        "data": { "object": {
            "id": "in_8", "customer": "cus_new", "amount_paid": 1490, "currency": "brl",
            "lines": { "data": [ { "period": { "end": now() + 30 * 86_400 } } ] }
        }}
    });
    apply_event(&server, &invoice).await.unwrap();
    assert_eq!(server.store.wallet(id).await.unwrap().vip_until, 0, "nobody to give it to yet");
    let session = json!({
        "type": "checkout.session.completed",
        "data": { "object": {
            "id": "cs_8", "mode": "subscription", "payment_status": "paid",
            "client_reference_id": id.to_string(), "customer": "cus_new",
            "invoice": "in_8", "amount_total": 1490, "currency": "brl"
        }}
    });
    apply_event(&server, &session).await.unwrap();
    apply_event(&server, &invoice).await.unwrap();
    apply_event(&server, &session).await.unwrap();
    let w = server.store.wallet(id).await.unwrap();
    assert!(w.vip_until > now() + 29 * 86_400);
    assert_eq!(w.coins, VIP_MONTHLY_COINS as i64, "one month of coins");
    // The subscription's own metadata on the invoice is enough, too.
    let renewal = json!({
        "type": "invoice.paid",
        "data": { "object": {
            "id": "in_9", "customer": "cus_other", "amount_paid": 1490, "currency": "brl",
            "parent": { "subscription_details": { "metadata": { "user_id": id.to_string() } } },
            "lines": { "data": [ { "period": { "end": now() + 60 * 86_400 } } ] }
        }}
    });
    apply_event(&server, &renewal).await.unwrap();
    let w = server.store.wallet(id).await.unwrap();
    assert!(w.vip_until > now() + 59 * 86_400);
    assert_eq!(w.coins, 2 * VIP_MONTHLY_COINS as i64);
}
