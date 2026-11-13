//! Real-money payments through Stripe: coin packs (one-off) and the VIP
//! subscription (monthly).
//!
//! The game never sees card data: the server asks Stripe for a hosted
//! payment page (Checkout), the player pays there (card, Pix, boleto…), and
//! Stripe tells the server through a signed webhook. Every credit is keyed
//! by Stripe's own ids, so a webhook delivered twice changes nothing.
//!
//! Configuration (all required to take payments):
//! - `SNOOKERED_STRIPE_SECRET`: the secret API key (`sk_live_…` / `sk_test_…`);
//! - `SNOOKERED_STRIPE_WEBHOOK_SECRET`: the webhook signing secret (`whsec_…`);
//! - `SNOOKERED_PUBLIC_URL`: the server's public address (for the pages Stripe
//!   sends the player back to);
//! - `SNOOKERED_STRIPE_PRICES`: `code=price_id|label` pairs separated by `;`,
//!   one per coin pack, `vip_monthly` and `vip_gift` (a one-off price: a
//!   month of VIP given to a friend), e.g.
//!   `coins_500=price_1Abc|R$ 4,90;vip_monthly=price_9Xyz|R$ 14,90/mês`.

use std::collections::HashMap;

use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

/// How old a signed webhook may be (replay protection).
const TOLERANCE_SECS: i64 = 300;

#[derive(Debug, Clone)]
pub struct StripeConfig {
    pub secret: String,
    pub webhook_secret: String,
    pub public_url: String,
    /// Product code -> (Stripe price id, price as shown to players).
    pub prices: HashMap<String, (String, String)>,
}

impl StripeConfig {
    /// From the environment; `None` (no payments) if anything is missing.
    pub fn from_env() -> Option<StripeConfig> {
        let var =
            |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let cfg = StripeConfig {
            secret: var("SNOOKERED_STRIPE_SECRET")?,
            webhook_secret: var("SNOOKERED_STRIPE_WEBHOOK_SECRET")?,
            public_url: var("SNOOKERED_PUBLIC_URL")?.trim_end_matches('/').to_string(),
            prices: parse_prices(&var("SNOOKERED_STRIPE_PRICES")?),
        };
        (!cfg.prices.is_empty()).then_some(cfg)
    }
}

/// `code=price_id|label;code=…`
pub fn parse_prices(text: &str) -> HashMap<String, (String, String)> {
    text.split(';')
        .filter_map(|entry| {
            let (code, rest) = entry.split_once('=')?;
            let (id, label) = rest.split_once('|').unwrap_or((rest, ""));
            let (code, id) = (code.trim(), id.trim());
            (!code.is_empty() && id.starts_with("price_"))
                .then(|| (code.to_string(), (id.to_string(), label.trim().to_string())))
        })
        .collect()
}

pub struct Stripe {
    pub cfg: StripeConfig,
    #[cfg(feature = "payments")]
    http: reqwest::Client,
}

impl Stripe {
    pub fn new(cfg: StripeConfig) -> Stripe {
        #[cfg(feature = "payments")]
        {
            // TLS with ring, like the database (no C toolchain needed).
            let _ = rustls::crypto::ring::default_provider().install_default();
            let http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("http client");
            Stripe { cfg, http }
        }
        #[cfg(not(feature = "payments"))]
        Stripe { cfg }
    }

    /// The price label of a product (empty if not on sale).
    pub fn label(&self, code: &str) -> Option<String> {
        self.cfg.prices.get(code).map(|(_, l)| l.clone())
    }

    #[cfg(not(feature = "payments"))]
    async fn post(&self, _path: &str, _form: &[(String, String)]) -> Result<Value, String> {
        Err("this build takes no payments".into())
    }

    #[cfg(feature = "payments")]
    async fn post(&self, path: &str, form: &[(String, String)]) -> Result<Value, String> {
        let res = self
            .http
            .post(format!("https://api.stripe.com/v1/{path}"))
            .bearer_auth(&self.cfg.secret)
            .form(form)
            .send()
            .await
            .map_err(|e| format!("payment provider unreachable: {e}"))?;
        let status = res.status();
        let body: Value = res.json().await.map_err(|e| format!("bad reply: {e}"))?;
        if !status.is_success() {
            let msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            return Err(format!("payment provider refused: {msg}"));
        }
        Ok(body)
    }

    fn return_urls(&self) -> [(String, String); 2] {
        [
            ("success_url".into(), format!("{}/shop/thanks", self.cfg.public_url)),
            ("cancel_url".into(), format!("{}/shop/cancelled", self.cfg.public_url)),
        ]
    }

    /// A payment page for a coin pack.
    pub async fn checkout_pack(
        &self,
        user_id: i64,
        username: &str,
        customer: Option<&str>,
        pack: &str,
    ) -> Result<String, String> {
        let (price, _) = self.cfg.prices.get(pack).ok_or("This pack is not on sale.")?;
        let mut form = vec![
            ("mode".to_string(), "payment".to_string()),
            ("line_items[0][price]".into(), price.clone()),
            ("line_items[0][quantity]".into(), "1".into()),
            ("client_reference_id".into(), user_id.to_string()),
            ("metadata[user_id]".into(), user_id.to_string()),
            ("metadata[username]".into(), username.to_string()),
            ("metadata[pack]".into(), pack.to_string()),
            ("payment_intent_data[metadata][user_id]".into(), user_id.to_string()),
            ("payment_intent_data[metadata][pack]".into(), pack.to_string()),
        ];
        match customer {
            Some(c) => form.push(("customer".into(), c.to_string())),
            None => form.push(("customer_creation".into(), "always".into())),
        }
        form.extend(self.return_urls());
        let session = self.post("checkout/sessions", &form).await?;
        session["url"].as_str().map(str::to_string).ok_or_else(|| "no payment page".into())
    }

    /// A payment page for the VIP subscription.
    pub async fn checkout_vip(
        &self,
        user_id: i64,
        username: &str,
        customer: Option<&str>,
    ) -> Result<String, String> {
        let (price, _) =
            self.cfg.prices.get(snookered_proto::shop::VIP_PLAN).ok_or("VIP is not on sale.")?;
        let mut form = vec![
            ("mode".to_string(), "subscription".to_string()),
            ("line_items[0][price]".into(), price.clone()),
            ("line_items[0][quantity]".into(), "1".into()),
            ("client_reference_id".into(), user_id.to_string()),
            ("metadata[user_id]".into(), user_id.to_string()),
            ("metadata[username]".into(), username.to_string()),
            ("subscription_data[metadata][user_id]".into(), user_id.to_string()),
        ];
        if let Some(c) = customer {
            form.push(("customer".into(), c.to_string()));
        }
        form.extend(self.return_urls());
        let session = self.post("checkout/sessions", &form).await?;
        session["url"].as_str().map(str::to_string).ok_or_else(|| "no payment page".into())
    }

    /// A payment page for a month of VIP given to `friend` (one payment).
    pub async fn checkout_vip_gift(
        &self,
        user_id: i64,
        username: &str,
        customer: Option<&str>,
        friend_id: i64,
    ) -> Result<String, String> {
        let (price, _) =
            self.cfg.prices.get(snookered_proto::shop::VIP_GIFT).ok_or("VIP can't be given.")?;
        let mut form = vec![
            ("mode".to_string(), "payment".to_string()),
            ("line_items[0][price]".into(), price.clone()),
            ("line_items[0][quantity]".into(), "1".into()),
            ("client_reference_id".into(), user_id.to_string()),
            ("metadata[user_id]".into(), user_id.to_string()),
            ("metadata[username]".into(), username.to_string()),
            ("metadata[gift_vip_to]".into(), friend_id.to_string()),
            ("payment_intent_data[metadata][user_id]".into(), user_id.to_string()),
            ("payment_intent_data[metadata][gift_vip_to]".into(), friend_id.to_string()),
        ];
        match customer {
            Some(c) => form.push(("customer".into(), c.to_string())),
            None => form.push(("customer_creation".into(), "always".into())),
        }
        form.extend(self.return_urls());
        let session = self.post("checkout/sessions", &form).await?;
        session["url"].as_str().map(str::to_string).ok_or_else(|| "no payment page".into())
    }

    /// The customer portal (change card, cancel the subscription, invoices).
    pub async fn portal(&self, customer: &str) -> Result<String, String> {
        let form = vec![
            ("customer".to_string(), customer.to_string()),
            ("return_url".into(), format!("{}/shop/thanks", self.cfg.public_url)),
        ];
        let session = self.post("billing_portal/sessions", &form).await?;
        session["url"].as_str().map(str::to_string).ok_or_else(|| "no portal page".into())
    }

    /// Checks a webhook's `Stripe-Signature` header against the payload.
    pub fn verify(&self, payload: &[u8], header: &str, now: i64) -> bool {
        verify_signature(&self.cfg.webhook_secret, payload, header, now)
    }
}

/// `t=…,v1=…[,v1=…]`: HMAC-SHA256 of `"{t}.{payload}"`, recent enough.
pub fn verify_signature(secret: &str, payload: &[u8], header: &str, now: i64) -> bool {
    let mut t = None;
    let mut sigs = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse::<i64>().ok(),
            Some(("v1", v)) => sigs.push(v.to_string()),
            _ => {}
        }
    }
    let Some(t) = t else { return false };
    if (now - t).abs() > TOLERANCE_SECS || sigs.is_empty() {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else { return false };
    mac.update(t.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    let expected = mac.finalize().into_bytes();
    sigs.iter().any(|s| {
        decode_hex(s).is_some_and(|got| {
            got.len() == expected.len()
                && got.iter().zip(expected.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
        })
    })
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, t: i64, payload: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{t}.{payload}").as_bytes());
        let hex: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
        format!("t={t},v1={hex}")
    }

    #[test]
    fn webhook_signatures() {
        let body = r#"{"id":"evt_1"}"#;
        let header = sign("whsec_test", 1000, body);
        assert!(verify_signature("whsec_test", body.as_bytes(), &header, 1100));
        assert!(!verify_signature("whsec_other", body.as_bytes(), &header, 1100), "wrong secret");
        assert!(!verify_signature("whsec_test", b"{}", &header, 1100), "changed body");
        assert!(!verify_signature("whsec_test", body.as_bytes(), &header, 2000), "too old");
        assert!(!verify_signature("whsec_test", body.as_bytes(), "t=1000", 1000), "no signature");
        assert!(!verify_signature("whsec_test", body.as_bytes(), "garbage", 1000));
    }

    #[test]
    fn price_list() {
        let p = parse_prices("coins_500=price_1A|R$ 4,90; vip_monthly=price_9Z|R$ 14,90/mês;bad=x");
        assert_eq!(p["coins_500"], ("price_1A".to_string(), "R$ 4,90".to_string()));
        assert_eq!(p["vip_monthly"].1, "R$ 14,90/mês");
        assert!(!p.contains_key("bad"));
    }
}
