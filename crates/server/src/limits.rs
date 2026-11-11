//! Rate limits against floods and password guessing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A token bucket: `burst` actions at once, refilled at `rate` per second.
#[derive(Debug, Clone)]
pub struct Bucket {
    tokens: f64,
    burst: f64,
    rate: f64,
    last: Instant,
}

impl Bucket {
    pub fn new(burst: f64, rate: f64) -> Bucket {
        Bucket { tokens: burst, burst, rate, last: Instant::now() }
    }

    /// Takes one token; false if the bucket is empty.
    pub fn take(&mut self) -> bool {
        let now = Instant::now();
        self.tokens =
            (self.tokens + now.duration_since(self.last).as_secs_f64() * self.rate).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Counts events per key inside a sliding window.
#[derive(Debug, Default)]
pub struct Window {
    events: HashMap<String, Vec<Instant>>,
}

impl Window {
    fn prune(list: &mut Vec<Instant>, window: Duration) {
        let now = Instant::now();
        list.retain(|t| now.duration_since(*t) < window);
    }

    pub fn count(&mut self, key: &str, window: Duration) -> usize {
        match self.events.get_mut(key) {
            Some(list) => {
                Self::prune(list, window);
                list.len()
            }
            None => 0,
        }
    }

    pub fn add(&mut self, key: &str) {
        self.events.entry(key.to_string()).or_default().push(Instant::now());
    }

    pub fn clear(&mut self, key: &str) {
        self.events.remove(key);
    }

    /// Drops keys with no recent events (call now and then).
    pub fn sweep(&mut self, window: Duration) {
        self.events.retain(|_, list| {
            Self::prune(list, window);
            !list.is_empty()
        });
    }
}

/// Failed sign-ins and new accounts, per address and per account.
#[derive(Debug, Default)]
pub struct AuthLimiter {
    failures_ip: Window,
    failures_user: Window,
    registrations: Window,
}

pub const AUTH_WINDOW: Duration = Duration::from_secs(15 * 60);
const REGISTER_WINDOW: Duration = Duration::from_secs(60 * 60);
const MAX_FAILURES_PER_IP: usize = 20;
const MAX_FAILURES_PER_USER: usize = 8;
const MAX_REGISTRATIONS_PER_IP: usize = 5;

impl AuthLimiter {
    /// True if another password attempt is allowed.
    pub fn may_try(&mut self, ip: &str, username: &str) -> bool {
        self.failures_ip.count(ip, AUTH_WINDOW) < MAX_FAILURES_PER_IP
            && self.failures_user.count(&username.to_lowercase(), AUTH_WINDOW)
                < MAX_FAILURES_PER_USER
    }

    pub fn failed(&mut self, ip: &str, username: &str) {
        self.failures_ip.add(ip);
        self.failures_user.add(&username.to_lowercase());
    }

    pub fn succeeded(&mut self, username: &str) {
        self.failures_user.clear(&username.to_lowercase());
    }

    pub fn may_register(&mut self, ip: &str) -> bool {
        self.registrations.count(ip, REGISTER_WINDOW) < MAX_REGISTRATIONS_PER_IP
    }

    pub fn registered(&mut self, ip: &str) {
        self.registrations.add(ip);
    }

    pub fn sweep(&mut self) {
        self.failures_ip.sweep(AUTH_WINDOW);
        self.failures_user.sweep(AUTH_WINDOW);
        self.registrations.sweep(REGISTER_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_limits_bursts() {
        let mut b = Bucket::new(3.0, 0.0);
        assert!(b.take() && b.take() && b.take());
        assert!(!b.take());
    }

    #[test]
    fn password_guessing_is_limited() {
        let mut l = AuthLimiter::default();
        for _ in 0..MAX_FAILURES_PER_USER {
            assert!(l.may_try("1.2.3.4", "Bob"));
            l.failed("1.2.3.4", "Bob");
        }
        assert!(!l.may_try("5.6.7.8", "bob"), "per account, from any address");
        assert!(l.may_try("1.2.3.4", "alice"));
    }
}
