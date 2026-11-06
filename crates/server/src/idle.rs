//! IDLE is only a wake-up hint. Durable arrival heads and Web Push jobs remain
//! the source of truth; the ordinary scheduler continues polling independently.
use crate::App;
use dakia_mail_provider::{self as p, MailProvider};
use dakia_storage::AuthorizedAccount;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::task::JoinHandle;

const MAX_ACCOUNTS: usize = 32;
const RENEW: Duration = Duration::from_secs(20 * 60);
const DEBOUNCE: Duration = Duration::from_secs(5);

struct Watcher {
    provider: Arc<dyn MailProvider>,
    task: JoinHandle<()>,
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One dedicated INBOX listener per ready account, never holding a command or
/// sync lane while waiting. Reconcile identity/settings/revocation every 5s.
/// Over-budget accounts retain normal polling, rather than unbounded sockets.
pub async fn run(app: App) {
    let mut watchers: HashMap<String, Watcher> = HashMap::new();
    loop {
        let accounts = if app.store.jobs_enabled().await.unwrap_or(false) {
            app.store.job_accounts().await.unwrap_or_default()
        } else {
            Vec::new()
        };
        watchers.retain(|id, _| accounts.iter().any(|a| a.id() == id));
        for a in accounts {
            let Ok(provider) = app.mail.provider(&app.store, &a).await else {
                watchers.remove(a.id());
                continue;
            };
            if watchers
                .get(a.id())
                .is_some_and(|w| !Arc::ptr_eq(&w.provider, &provider))
            {
                watchers.remove(a.id());
            }
            // A finished authentication-failed listener is retained until its
            // account/provider changes, not restarted into a login storm.
            if watchers.contains_key(a.id()) || watchers.len() >= MAX_ACCOUNTS {
                continue;
            }
            let id = a.id().to_owned();
            let task = tokio::spawn(listen(app.clone(), a, provider.clone()));
            watchers.insert(id, Watcher { provider, task });
        }
        tokio::time::sleep(DEBOUNCE).await;
    }
}

async fn head(app: &App, a: &AuthorizedAccount) {
    let lane = app.mail.sync_lane(a).await;
    let _guard = lane.lock().await;
    let result = app.mail.check_head(&app.store, a, "INBOX").await;
    match result {
        Ok(true) => {
            let _ = app.store.inbox_refresh_result(a, None).await;
        }
        Err(error) => {
            let _ = app.store.inbox_refresh_result(a, Some(error.1)).await;
        }
        Ok(false) => {}
    }
}

async fn listen(app: App, a: AuthorizedAccount, provider: Arc<dyn MailProvider>) {
    let mut failures = 0;
    // Recover arrivals missed while offline. A head never finishes/prunes the
    // historical snapshot and initial history remains notification-suppressed.
    head(&app, &a).await;
    loop {
        let result = provider.wait_for_change("INBOX", RENEW).await;
        // Retirement intentionally interrupts IDLE with Authentication. Never
        // mark a newly reconnected account broken because its old socket ended.
        if !app
            .mail
            .provider(&app.store, &a)
            .await
            .is_ok_and(|current| Arc::ptr_eq(&provider, &current))
        {
            return;
        }
        let delay = retry_delay(&result, &mut failures);
        let Some(delay) = delay else {
            let _ = app
                .store
                .account_state(&a, dakia_domain::AccountState::Reconnect)
                .await;
            return;
        };
        // One bounded wake-up per burst, including duplicate EXISTS/FETCH
        // hints. Delay also respects the shared head polling budget.
        tokio::time::sleep(delay).await;
        head(&app, &a).await;
    }
}

fn retry_delay(result: &p::Result<bool>, failures: &mut u32) -> Option<Duration> {
    match result {
        Ok(_) => {
            *failures = 0;
            Some(DEBOUNCE)
        }
        Err(e) if e.kind == p::ErrorKind::Authentication => None,
        // Servers without IDLE use existing polling; recheck capabilities after
        // 15 minutes, so an operator/provider change needs no process restart.
        Err(e) if e.kind == p::ErrorKind::Unsupported => Some(Duration::from_secs(900)),
        Err(_) => {
            *failures = failures.saturating_add(1);
            Some(Duration::from_secs((5_u64 << (*failures).min(6)).min(300)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hints_timeouts_unsupported_disconnects_and_auth_have_bounded_policy() {
        let mut failures = 0;
        for _ in 0..20 {
            assert!(
                retry_delay(
                    &Err(p::error(p::ErrorKind::Retryable, "fixture")),
                    &mut failures
                )
                .unwrap()
                    <= Duration::from_secs(300)
            );
        }
        assert_eq!(retry_delay(&Ok(false), &mut failures), Some(DEBOUNCE));
        assert_eq!(failures, 0);
        assert_eq!(retry_delay(&Ok(true), &mut failures), Some(DEBOUNCE));
        assert_eq!(
            retry_delay(
                &Err(p::error(p::ErrorKind::Unsupported, "fixture")),
                &mut failures
            ),
            Some(Duration::from_secs(900))
        );
        assert_eq!(
            retry_delay(
                &Err(p::error(p::ErrorKind::Authentication, "fixture")),
                &mut failures
            ),
            None
        );
    }
}
