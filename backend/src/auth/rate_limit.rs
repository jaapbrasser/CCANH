//! Login rate limiting with exponential lockout, keyed per username and per IP.
//!
//! After `free_attempts` consecutive failures for a key, further attempts are
//! refused for `base_delay * 2^(extra failures)`, capped at `max_delay`. A key's
//! history is forgotten after `forget_after` without failures, and a successful
//! login clears the username key. State is in memory: a restart resets it, which
//! is acceptable for a LAN service and avoids a write per failed attempt.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Policy {
    pub free_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub forget_after: Duration,
}

impl Policy {
    /// Per-username policy: 5 free attempts, then 1 s, 2 s, 4 s … up to 15 min.
    pub fn username() -> Self {
        Self {
            free_attempts: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(15 * 60),
            forget_after: Duration::from_secs(60 * 60),
        }
    }

    /// Per-IP policy: more generous (several users may share a NAT'd address).
    pub fn ip() -> Self {
        Self {
            free_attempts: 20,
            ..Self::username()
        }
    }

    fn delay_for(&self, failures: u32) -> Option<Duration> {
        // The first failure past the free allowance waits `base_delay`.
        let extra = failures.checked_sub(self.free_attempts.saturating_add(1))?;
        let factor = 2u32.saturating_pow(extra.min(31));
        Some(self.base_delay.saturating_mul(factor).min(self.max_delay))
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    last_failure: Instant,
    locked_until: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    User(String),
    Ip(IpAddr),
}

const MAX_ENTRIES: usize = 10_000;

#[derive(Debug)]
pub struct LoginLimiter {
    user_policy: Policy,
    ip_policy: Policy,
    entries: Mutex<HashMap<Key, Entry>>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new(Policy::username(), Policy::ip())
    }
}

impl LoginLimiter {
    pub fn new(user_policy: Policy, ip_policy: Policy) -> Self {
        Self {
            user_policy,
            ip_policy,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn keys(username: &str, ip: Option<IpAddr>) -> impl Iterator<Item = Key> {
        std::iter::once(Key::User(username.to_lowercase())).chain(ip.map(Key::Ip))
    }

    fn policy(&self, key: &Key) -> &Policy {
        match key {
            Key::User(_) => &self.user_policy,
            Key::Ip(_) => &self.ip_policy,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Entry>> {
        // A poisoned lock only means another thread panicked mid-update; the map
        // is still usable for rate limiting.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Err(retry_after)` if either the username or the IP is currently locked.
    pub fn check(&self, username: &str, ip: Option<IpAddr>) -> Result<(), Duration> {
        self.check_at(username, ip, Instant::now())
    }

    pub fn record_failure(&self, username: &str, ip: Option<IpAddr>) {
        self.record_failure_at(username, ip, Instant::now());
    }

    /// Clear the username's history after a successful login. The IP history is
    /// kept, so one valid account can't be used to reset an IP's lockout.
    pub fn record_success(&self, username: &str) {
        self.lock().remove(&Key::User(username.to_lowercase()));
    }

    fn check_at(&self, username: &str, ip: Option<IpAddr>, now: Instant) -> Result<(), Duration> {
        let entries = self.lock();
        let mut wait = Duration::ZERO;
        for key in Self::keys(username, ip) {
            if let Some(until) = entries.get(&key).and_then(|e| e.locked_until) {
                wait = wait.max(until.saturating_duration_since(now));
            }
        }
        if wait.is_zero() {
            Ok(())
        } else {
            Err(wait)
        }
    }

    fn record_failure_at(&self, username: &str, ip: Option<IpAddr>, now: Instant) {
        let mut entries = self.lock();
        if entries.len() >= MAX_ENTRIES {
            let forget = self
                .user_policy
                .forget_after
                .max(self.ip_policy.forget_after);
            entries.retain(|_, e| now.saturating_duration_since(e.last_failure) < forget);
        }
        for key in Self::keys(username, ip) {
            let policy = self.policy(&key).clone();
            let entry = entries.entry(key).or_insert(Entry {
                failures: 0,
                last_failure: now,
                locked_until: None,
            });
            if now.saturating_duration_since(entry.last_failure) >= policy.forget_after {
                entry.failures = 0;
            }
            entry.failures = entry.failures.saturating_add(1);
            entry.last_failure = now;
            entry.locked_until = policy.delay_for(entry.failures).map(|d| now + d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> Option<IpAddr> {
        Some(IpAddr::from([10, 0, 0, last]))
    }

    #[test]
    fn allows_free_attempts_then_locks_with_exponential_delay() {
        let limiter = LoginLimiter::default();
        let t0 = Instant::now();
        for _ in 0..5 {
            assert!(limiter.check_at("alice", ip(1), t0).is_ok());
            limiter.record_failure_at("alice", ip(1), t0);
        }
        // 5 failures: still within free attempts.
        assert!(limiter.check_at("alice", ip(1), t0).is_ok());

        limiter.record_failure_at("alice", ip(1), t0); // 6th → 1 s
        assert_eq!(
            limiter.check_at("alice", ip(1), t0),
            Err(Duration::from_secs(1))
        );
        let t1 = t0 + Duration::from_secs(1);
        assert!(limiter.check_at("alice", ip(1), t1).is_ok());

        limiter.record_failure_at("alice", ip(1), t1); // 7th → 2 s
        assert_eq!(
            limiter.check_at("alice", ip(1), t1),
            Err(Duration::from_secs(2))
        );
    }

    #[test]
    fn delay_is_capped() {
        let policy = Policy::username();
        assert_eq!(policy.delay_for(1000), Some(policy.max_delay));
        assert_eq!(policy.delay_for(5), None);
        assert_eq!(policy.delay_for(6), Some(policy.base_delay));
    }

    #[test]
    fn username_key_is_case_insensitive() {
        let limiter = LoginLimiter::default();
        let t = Instant::now();
        for _ in 0..6 {
            limiter.record_failure_at("Alice", None, t);
        }
        assert!(limiter.check_at("alice", None, t).is_err());
    }

    #[test]
    fn success_clears_username_but_not_ip() {
        let limiter = LoginLimiter::new(
            Policy::username(),
            Policy {
                free_attempts: 2,
                ..Policy::ip()
            },
        );
        let t = Instant::now();
        for _ in 0..6 {
            limiter.record_failure_at("alice", ip(1), t);
        }
        limiter.record_success("alice");
        // Username cleared, but the IP is still locked.
        assert!(limiter.check_at("alice", ip(2), t).is_ok());
        assert!(limiter.check_at("bob", ip(1), t).is_err());
    }

    #[test]
    fn per_ip_lock_applies_across_usernames() {
        let limiter = LoginLimiter::default();
        let t = Instant::now();
        for i in 0..21 {
            limiter.record_failure_at(&format!("user{i}"), ip(9), t);
        }
        assert!(limiter.check_at("someone-else", ip(9), t).is_err());
        assert!(limiter.check_at("someone-else", ip(8), t).is_ok());
    }

    #[test]
    fn history_is_forgotten_after_quiet_period() {
        let limiter = LoginLimiter::default();
        let t0 = Instant::now();
        for _ in 0..5 {
            limiter.record_failure_at("alice", None, t0);
        }
        let later = t0 + Duration::from_secs(2 * 60 * 60);
        limiter.record_failure_at("alice", None, later);
        assert!(limiter.check_at("alice", None, later).is_ok());
    }
}
