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
//! | `SNOOK_CLIENT_IP_HEADER` | — | header with the real client address behind a proxy |
//! | `SNOOK_PROXY_HOPS` | `1` | proxies that append to `X-Forwarded-For` |
//! | `SNOOK_MOTD` | a welcome line | message shown in the lounge |
//! | `SNOOK_SHOT_CLOCK` | `30` | seconds per shot |
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
    };
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
    tokio::select! {
        result = serve(listener, server) => {
            if let Err(e) = result {
                tracing::error!("server stopped: {e}");
            }
        }
        _ = shutdown => tracing::info!("shutting down"),
    }
}
