//! Snook server executable.
//!
//! Configuration (environment variables):
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `SNOOK_BIND` | `0.0.0.0:7878` | address and port to listen on |
//! | `SNOOK_DB` | `snook.db` | SQLite database file |
//! | `SNOOK_MOTD` | a welcome line | message shown to players after login |
//! | `SNOOK_SHOT_CLOCK` | `30` | seconds per shot (then the shot is played at full power) |
//! | `RUST_LOG` | `info` | log filter |

use std::path::PathBuf;
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
    if let Some(secs) = env("SNOOK_SHOT_CLOCK").and_then(|s| s.parse().ok()) {
        rules.shot_clock = Duration::from_secs(secs);
    }
    let config = Config {
        bind: env("SNOOK_BIND")
            .unwrap_or_else(|| "0.0.0.0:7878".into())
            .parse()
            .expect("SNOOK_BIND must be host:port"),
        database: Some(PathBuf::from(env("SNOOK_DB").unwrap_or_else(|| "snook.db".into()))),
        rules,
    };
    let server = match Server::new(&config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not open the database: {e}");
            std::process::exit(1);
        }
    };
    let listener = TcpListener::bind(config.bind).await.expect("could not bind the server address");
    tracing::info!("Snook server listening on {} (WebSocket at /ws)", config.bind);
    tokio::select! {
        result = serve(listener, server) => {
            if let Err(e) = result {
                tracing::error!("server stopped: {e}");
            }
        }
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
    }
}
