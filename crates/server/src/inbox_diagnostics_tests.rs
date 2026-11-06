use super::*;
use crate::diagnostics::{retained_target, Retained};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

#[tokio::test]
async fn refresh_logs_correlate_failure_and_recovery_without_private_data() {
    // Isolate the global dispatcher used by spawned background refresh tasks.
    const CHILD: &str = "DAKIA_REFRESH_LOG_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "mail::integration_tests::inbox_tests::diagnostics_tests::refresh_logs_correlate_failure_and_recovery_without_private_data", "--nocapture"])
            .env(CHILD, "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (dir, app, user, a, provider) = setup(1, 10).await;
    let retained = Retained::open(dir.path()).unwrap();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(retained)
                .with_filter(tracing_subscriber::filter::filter_fn(|m| {
                    retained_target(m.target())
                })),
        )
        .init();
    tracing::warn!(target: "untrusted_dependency", "private payload should not be retained: private@example.invalid");
    provider.fail.store(true, Ordering::SeqCst);
    let req = serde_json::json!({"q":"folder:INBOX", "accounts":a.id()});
    search(&app, &user, req.clone()).await.unwrap();
    wait_refresh(&app, &a).await;
    assert_eq!(
        app.store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Error
    );
    provider.fail.store(false, Ordering::SeqCst);
    let slot = app
        .mail
        .inbox_refreshes
        .lock()
        .await
        .get(a.id())
        .unwrap()
        .clone();
    *slot.lock().await = None;
    app.mail.refresh_inbox(&app.store, &a, false).await;
    wait_refresh(&app, &a).await;
    assert!(app.store.inbox_coverage(&a).await.unwrap().error.is_none());
    let text = std::fs::read_to_string(dir.path().join("diagnostics/mail-events.0.log")).unwrap();
    for expected in [
        "inbox_refresh",
        "account_key=",
        "refresh_key=",
        "stage=\"scan_page\"",
        "reason=\"provider_unavailable\"",
        "outcome=\"failed\"",
        "outcome=\"success\"",
        "lane_wait_ms=",
        "elapsed_ms=",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    for private in [
        a.id(),
        "owner@example.invalid",
        "fixture failure",
        "private@example.invalid",
        "fixture-only",
        "Synthetic 0",
    ] {
        assert!(!text.contains(private), "private data leaked");
    }
    let refresh_keys: std::collections::HashSet<_> = text
        .split("refresh_key=")
        .skip(1)
        .map(|s| s.split('}').next().unwrap())
        .collect();
    assert_eq!(
        refresh_keys.len(),
        2,
        "each attempt has its own correlation"
    );
}
