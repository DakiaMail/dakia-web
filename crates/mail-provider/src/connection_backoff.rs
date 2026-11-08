//! Account-wide connection admission, shared by command and IDLE lanes.
use crate::{error, ErrorKind, Result};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct ConnectionBackoff {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    failures: u32,
    until: Option<Instant>,
    active: bool,
}
impl ConnectionBackoff {
    pub(super) fn begin(&self, account: &str) -> Result<Attempt<'_>> {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        if state.active || state.until.is_some_and(|until| until > now) {
            let wait_ms = state.until.map_or(0, |until| {
                until.saturating_duration_since(now).as_millis() as u64
            });
            tracing::info!(target: "dakia_server::provider_io", account_key = %crate::telemetry::key(account), operation = "connection_backoff", outcome = "skipped", wait_ms, "provider_io");
            return Err(error(ErrorKind::Retryable, "IMAP reconnect cooling down"));
        }
        state.failures = state.failures.saturating_add(1);
        let delay = Duration::from_secs((5u64 << state.failures.saturating_sub(1).min(6)).min(300));
        state.until = Some(now + delay);
        state.active = true;
        tracing::info!(target: "dakia_server::provider_io", account_key = %crate::telemetry::key(account), reused = false, "imap_connection");
        Ok(Attempt {
            backoff: self,
            account_key: crate::telemetry::key(account),
            stage: "connect",
            outcome: "cancelled",
            reason: "interrupted",
            started: now,
        })
    }
    /// Reset the failure run only after a clean command or initialized IDLE.
    pub(super) fn recovered(&self) {
        let mut state = self.state.lock().unwrap();
        state.failures = 0;
        state.until = None;
    }
    #[cfg(test)]
    pub(super) fn expire(&self) {
        self.state.lock().unwrap().until = None;
    }
}

pub(super) struct Attempt<'a> {
    backoff: &'a ConnectionBackoff,
    account_key: String,
    stage: &'static str,
    outcome: &'static str,
    reason: &'static str,
    started: Instant,
}
impl Attempt<'_> {
    pub(super) fn reason(&mut self, reason: &'static str) {
        self.reason = reason;
    }
    pub(super) fn stage(&mut self, stage: &'static str) {
        self.stage = stage;
    }
    pub(super) fn finished<T>(&mut self, result: &Result<T>) {
        if self.reason == "interrupted" {
            self.reason = match result {
                Ok(_) => "none",
                Err(e) if e.kind == ErrorKind::Authentication => "authentication_rejected",
                Err(_) => "handshake_failed",
            };
        }
        self.outcome = if result.is_ok() {
            "authenticated"
        } else {
            "failed"
        };
    }
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        let mut state = self.backoff.state.lock().unwrap();
        state.active = false;
        // Cooldown starts after failure/cancellation, including slow handshakes.
        if self.outcome == "authenticated" {
            state.until = None;
        } else {
            let delay =
                Duration::from_secs((5u64 << state.failures.saturating_sub(1).min(6)).min(300));
            state.until = Some(Instant::now() + delay);
        }
        tracing::info!(target: "dakia_server::provider_io", account_key = %self.account_key, operation = "connection_attempt", stage = self.stage, reason = self.reason, outcome = self.outcome, elapsed_ms = self.started.elapsed().as_millis() as u64, "provider_io");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_and_cancelled_attempts_back_off_exponentially_and_recover() {
        let b = ConnectionBackoff::default();
        for seconds in [5, 10, 20, 40, 80, 160, 300, 300] {
            let attempt = b.begin("fixture").unwrap();
            assert!(b.begin("fixture").is_err());
            drop(attempt);
            let state = b.state.lock().unwrap();
            let remaining = state
                .until
                .unwrap()
                .saturating_duration_since(Instant::now());
            assert!(remaining <= Duration::from_secs(seconds));
            assert!(remaining > Duration::from_secs(seconds - 1));
            drop(state);
            assert!(b.begin("fixture").is_err());
            b.expire();
        }
        b.recovered();
        assert_eq!(b.state.lock().unwrap().failures, 0);
        assert!(b.begin("fixture").is_ok());
        assert!(ConnectionBackoff::default().begin("other-account").is_ok());
    }
}
