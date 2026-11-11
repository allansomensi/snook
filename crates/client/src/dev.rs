//! Development switches read from the environment (automated matches,
//! screenshots). Release builds ignore them all: one of them lets the
//! computer play a seat online, which must never reach players.

/// The variable's value in development builds; always `None` in release.
pub fn var(name: &str) -> Option<String> {
    if cfg!(debug_assertions) { std::env::var(name).ok() } else { None }
}

/// Whether the variable is set (development builds only).
pub fn flag(name: &str) -> bool {
    var(name).is_some()
}
