//! Snook server executable.
//!
//! Configuration (environment variables):
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `PORT` / `SNOOK_BIND` | `0.0.0.0:7878` | port (`PORT`, as set by hosts like Render) or full address |
//! | `DATABASE_URL` | — | PostgreSQL connection string (required unless `SNOOK_IN_MEMORY=1`) |
//! | `SNOOK_DB_POOL` | `5` | database connections |
//! | `SNOOK_IN_MEMORY` | — | `1`: keep everything in memory (development only) |
//! | `SNOOK_ADMINS` | — | comma-separated usernames that become administrators |
//! | `SNOOK_CLIENT_IP_HEADER` | — | headers with the real client address behind a proxy, tried in order (e.g. `cf-connecting-ip,true-client-ip,x-forwarded-for`) |
//! | `SNOOK_PROXY_HOPS` | `1` | proxies that append to `X-Forwarded-For` |
//! | `SNOOK_MOTD` | a welcome line | message shown in the lounge |
//! | `SNOOK_SHOT_CLOCK` | `30` | seconds per shot |
//! | `SNOOK_STRIPE_SECRET` | — | Stripe secret key (payments; see `payments.rs`) |
//! | `SNOOK_STRIPE_WEBHOOK_SECRET` | — | Stripe webhook signing secret |
//! | `SNOOK_STRIPE_PRICES` | — | `code=price_id\|label;…` for coin packs and `vip_monthly` |
//! | `SNOOK_PUBLIC_URL` | — | the server's public address (payment return pages) |
//! | `SNOOK_DRAIN_SECS` | `240` | on SIGTERM: how long games in progress may go on before they are handed to the next server |
//! | `SNOOK_CUP` | — | the weekly cup, e.g. `sat 20:00` (options: `60m`, `fee=100`, `utc-3`) |
//! | `RUST_LOG` | `info` | log filter |

use std::time::Duration;

use snook_server::lounge::Rules;
use snook_server::{Config, Server, serve};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let mut rules = Rules::default();
    if let Some(motd) = env("SNOOK_MOTD") {
        rules.motd = motd;
    }
    if let Some(secs) = env("SNOOK_SHOT_CLOCK").and_then(|s| s.parse::<u64>().ok()) {
        rules.shot_clock = Duration::from_secs(secs.clamp(10, 120));
    }
    if let Some(cup) = env("SNOOK_CUP") {
        rules.cup = snook_server::lounge::Cup::parse(&cup);
        match &rules.cup {
            Some(c) => tracing::info!("weekly cup: {c:?}"),
            None => tracing::warn!("SNOOK_CUP not understood: {cup}"),
        }
    }
    let drain_secs = env("SNOOK_DRAIN_SECS").and_then(|s| s.parse::<u64>().ok()).unwrap_or(240);
    let bind = match (env("SNOOK_BIND"), env("PORT")) {
        (Some(b), _) => b,
        (None, Some(port)) => format!("0.0.0.0:{port}"),
        (None, None) => "0.0.0.0:7878".into(),
    };
    let in_memory = env("SNOOK_IN_MEMORY").is_some_and(|v| v == "1" || v == "true");
    let database_url = env("DATABASE_URL");
    if database_url.is_none() && !in_memory {
        eprintln!("DATABASE_URL is not set (use SNOOK_IN_MEMORY=1 to run without a database)");
        std::process::exit(1);
    }
    let config = Config {
        bind: bind.parse().expect("SNOOK_BIND must be host:port"),
        database_url: if in_memory { None } else { database_url },
        database_pool: env("SNOOK_DB_POOL").and_then(|v| v.parse().ok()).unwrap_or(5),
        rules,
        admins: env("SNOOK_ADMINS")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default(),
        client_ip_header: env("SNOOK_CLIENT_IP_HEADER").map(|h| h.to_lowercase()),
        proxy_hops: env("SNOOK_PROXY_HOPS").and_then(|v| v.parse().ok()).unwrap_or(1),
        stripe: snook_server::payments::StripeConfig::from_env(),
    };
    match &config.stripe {
        Some(s) => tracing::info!("payments on ({} prices)", s.prices.len()),
        None => tracing::info!("payments off (SNOOK_STRIPE_* not set)"),
    }
    if in_memory {
        tracing::warn!("running in memory: accounts and games are lost when the server stops");
    }
    let bind = config.bind;
    let server = match Server::new(config).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not start: {e}");
            std::process::exit(1);
        }
    };
    let listener = TcpListener::bind(bind).await.expect("could not bind the server address");
    tracing::info!("Snook server {} listening on {bind}", env!("CARGO_PKG_VERSION"));
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal handler");
            tokio::select! {
                _ = ctrl_c => {}
                _ = term.recv() => {}
            }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
    };
    let serving = tokio::spawn(serve(listener, server.clone()));
    tokio::select! {
        result = serving => {
            if let Ok(Err(e)) = result {
                tracing::error!("server stopped: {e}");
            }
        }
        _ = shutdown => {
            // Keep serving while the games in progress finish.
            tracing::info!("shutting down: draining for up to {drain_secs} s");
            snook_server::drain(server, drain_secs).await;
        }
    }
}
