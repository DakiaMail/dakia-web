use chrono::{Duration, Utc};
use dakia_domain::*;
use dakia_storage::{Error, Store};

fn block(email: &str) -> CreateRule {
    CreateRule {
        preset: RulePreset::BlockSender,
        scope: RuleScope::AllMailboxes,
        condition: RuleCondition::SenderIs {
            email: email.into(),
        },
        actions: vec![RuleAction::Trash, RuleAction::SuppressNotifications],
    }
}
fn input(email: &str) -> ConnectAccount {
    serde_json::from_value(serde_json::json!({"email":email,"display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap()
}
fn mail(account: &str, id: &str, folder: &str, received: &str) -> Message {
    serde_json::from_value(serde_json::json!({"id":id,"account_id":account,"thread_id":id,"provider_id":id,"folder_ids":[folder],"sender":{"email":"Offers@Example.org","name":"Offers"},"to":[],"subject":"Disposable offer","preview":"Sample","received_at":received,"unread":true,"starred":false,"category":"people","category_status":"ready","revision":0})).unwrap()
}
async fn setup() -> (tempfile::TempDir, Store, String, String, Account) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path(), [42; 32]).await.unwrap();
    let user = s.user("fixture", "owner").await.unwrap();
    let other = s.user("fixture", "other").await.unwrap();
    let account = s
        .connect(&user, &input("owner@example.invalid"))
        .await
        .unwrap();
    (d, s, user, other, account)
}
fn future() -> String {
    (Utc::now() + Duration::seconds(20)).to_rfc3339()
}

#[tokio::test]
async fn rules_are_normalized_idempotent_owned_and_persistent() {
    let (d, s, user, other, _) = setup().await;
    let rule = s
        .create_rule(&user, block("  OFFERS@example.org "))
        .await
        .unwrap();
    assert_eq!(
        rule.condition,
        RuleCondition::SenderIs {
            email: "offers@example.org".into()
        }
    );
    assert_eq!(
        rule,
        s.create_rule(&user, block("offers@example.org"))
            .await
            .unwrap()
    );
    assert!(s.rules(&other).await.unwrap().is_empty());
    assert!(matches!(
        s.update_rule(&other, &rule.id, UpdateRule { enabled: false })
            .await,
        Err(Error::NotFound)
    ));
    for email in [
        "",
        "@example.org",
        "user@",
        "x@y@z",
        "x\ny@example.org",
        "Name <x@y.org>",
    ] {
        assert!(matches!(
            s.create_rule(&user, block(email)).await,
            Err(Error::Invalid)
        ));
    }
    let mut unsupported = block("other@example.org");
    unsupported.actions.pop();
    assert!(matches!(
        s.create_rule(&user, unsupported).await,
        Err(Error::Invalid)
    ));
    drop(s);
    let s = Store::open(d.path(), [42; 32]).await.unwrap();
    assert_eq!(s.rules(&user).await.unwrap(), vec![rule.clone()]);
    let disabled = s
        .update_rule(&user, &rule.id, UpdateRule { enabled: false })
        .await
        .unwrap();
    assert!(!disabled.enabled);
    let enabled = s
        .create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    assert_eq!(enabled.id, rule.id);
    assert!(enabled.activated_at >= rule.activated_at);
}

#[tokio::test]
async fn arrivals_queue_once_and_restored_messages_stay_restored_after_restart() {
    let (d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    let rule = s
        .create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    let m = mail(&account.id, "arrival", "INBOX", &future());
    s.ingest_messages(&a, vec![m.clone()], &[]).await.unwrap();
    s.ingest_messages(&a, vec![m.clone()], &[]).await.unwrap();
    assert_eq!(s.rule_executions(&a).await.unwrap().len(), 1);
    let projected = s
        .visible_message(&a, s.message(&a, &m.id).await.unwrap())
        .await
        .unwrap();
    assert_eq!(projected.folder_ids, vec!["Trash"]);
    let mut op = s.claim_mutation(&a).await.unwrap().unwrap();
    assert!(op.id.starts_with(&format!("rule:{}:", rule.id)));
    op.status = OperationStatus::Applied;
    s.finish_mutation(&a, op, Some(vec!["Trash".into()]))
        .await
        .unwrap();
    s.ingest_messages(&a, vec![m.clone()], &[]).await.unwrap(); // restored at provider
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    drop(s);
    let s = Store::open(d.path(), [42; 32]).await.unwrap();
    let a = s.authorize(&user, &account.id).await.unwrap();
    s.ingest_messages(&a, vec![m], &[]).await.unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn historical_outgoing_trash_other_senders_and_existing_mail_are_untouched() {
    let (_d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    let date = future();
    s.upsert_message(&a, mail(&account.id, "already-open", "INBOX", &date))
        .await
        .unwrap();
    s.create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    let old = (Utc::now() - Duration::days(1)).to_rfc3339();
    for (id, folder, at) in [
        ("already-open", "INBOX", date.as_str()),
        ("old", "INBOX", old.as_str()),
        ("sent", "Sent", date.as_str()),
        ("draft", "Drafts", date.as_str()),
        ("trash", "Trash", date.as_str()),
    ] {
        s.upsert_message(&a, mail(&account.id, id, folder, at))
            .await
            .unwrap();
    }
    let mut m = mail(&account.id, "substring", "INBOX", &date);
    m.sender.email = "xoffers@example.org".into();
    s.upsert_message(&a, m).await.unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    // Gmail-style discovery outside Inbox must not prevent its first arrival.
    s.upsert_message(&a, mail(&account.id, "allmail-first", "All Mail", &date))
        .await
        .unwrap();
    s.upsert_message(&a, mail(&account.id, "allmail-first", "INBOX", &date))
        .await
        .unwrap();
    assert_eq!(
        s.claim_mutation(&a).await.unwrap().unwrap().target_id,
        "allmail-first"
    );
}

#[tokio::test]
async fn new_accounts_and_users_are_scoped_and_connection_backfill_is_quiet() {
    let (_d, s, user, other, _) = setup().await;
    s.create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    let old = Utc::now().to_rfc3339();
    let account = s
        .connect(&user, &input("second@example.invalid"))
        .await
        .unwrap();
    let a = s.authorize(&user, &account.id).await.unwrap();
    s.upsert_message(&a, mail(&account.id, "backfill", "INBOX", &old))
        .await
        .unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    s.upsert_message(&a, mail(&account.id, "new", "INBOX", &future()))
        .await
        .unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_some());
    let account = s
        .connect(&other, &input("other@example.invalid"))
        .await
        .unwrap();
    let a = s.authorize(&other, &account.id).await.unwrap();
    s.upsert_message(&a, mail(&account.id, "other-user", "INBOX", &future()))
        .await
        .unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn disabling_cancels_only_unstarted_rule_moves_and_recovery_never_replays() {
    let (_d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    let rule = s
        .create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    for id in ["running", "pending"] {
        s.upsert_message(&a, mail(&account.id, id, "INBOX", &future()))
            .await
            .unwrap();
    }
    let running = s.claim_mutation(&a).await.unwrap().unwrap();
    s.update_rule(&user, &rule.id, UpdateRule { enabled: false })
        .await
        .unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    assert_eq!(s.rule_executions(&a).await.unwrap().len(), 1);
    s.recover_mutations(&a).await.unwrap();
    assert_eq!(
        s.operation_status(&a, &running.id).await.unwrap().status,
        OperationStatus::Failed
    );
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    assert_eq!(
        s.visible_message(&a, s.message(&a, "pending").await.unwrap())
            .await
            .unwrap()
            .folder_ids,
        vec!["INBOX"]
    );
}

#[tokio::test]
async fn conflicting_moves_are_reported_without_a_second_provider_action() {
    let (_d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    // Discover outside Inbox, then queue a deliberate move before Inbox arrival.
    let m = s
        .upsert_message(&a, mail(&account.id, "conflict", "Archive", &future()))
        .await
        .unwrap();
    s.operation(
        &a,
        Operation {
            id: "manual".into(),
            account_id: account.id.clone(),
            target_id: m.id.clone(),
            expected_revision: m.revision,
            change: Change::Archive,
            status: OperationStatus::Pending,
            revision: 0,
        },
    )
    .await
    .unwrap();
    s.create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    s.upsert_message(&a, mail(&account.id, "conflict", "INBOX", &future()))
        .await
        .unwrap();
    let rows = s.rule_executions(&a).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "conflict");
    assert_eq!(s.claim_mutation(&a).await.unwrap().unwrap().id, "manual");
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn push_is_suppressed_at_arrival_claim_and_final_delivery() {
    let (d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    s.subscribe(&user, "endpoint", r#"{"id":"endpoint"}"#)
        .await
        .unwrap();
    s.record_incremental(&a, "INBOX", &[], "bootstrap")
        .await
        .unwrap();
    let rule = s
        .create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    let m = mail(&account.id, "blocked-arrival", "INBOX", &future());
    s.ingest_messages(&a, vec![m.clone()], &[]).await.unwrap();
    s.record_incremental(&a, "INBOX", &[m], "next")
        .await
        .unwrap();
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        d.path()
            .join("accounts")
            .join(format!("{}.sqlite", account.id))
            .display()
    ))
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM push_jobs")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
    s.update_rule(&user, &rule.id, UpdateRule { enabled: false })
        .await
        .unwrap();
    let m = mail(&account.id, "queued-before-block", "INBOX", &future());
    s.ingest_messages(&a, vec![m.clone()], &[]).await.unwrap();
    s.record_incremental(&a, "INBOX", &[m], "next2")
        .await
        .unwrap();
    assert!(s.next_push(&a).await.unwrap().is_some());
    s.create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    assert!(!s
        .push_send_allowed(&a, "queued-before-block", "endpoint")
        .await
        .unwrap());
    let state: String =
        sqlx::query_scalar("SELECT state FROM push_jobs WHERE message_id='queued-before-block'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(state, "skipped");
    // Pending/recovered jobs must also be skipped by next_push.
    sqlx::query("UPDATE push_jobs SET state='pending'")
        .execute(&db)
        .await
        .unwrap();
    assert!(s.next_push(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn saved_localized_folder_mappings_exclude_outgoing_and_trash() {
    let (_d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    s.create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    s.put_provider_document(
        &a,
        "mailbox-settings",
        r#"{"trash_folder":"Papierkorb","sent_folder":"Gesendet"}"#,
    )
    .await
    .unwrap();
    for folder in ["Papierkorb", "Gesendet"] {
        let mut m = mail(&account.id, folder, "INBOX", &future());
        m.folder_ids.push(folder.into());
        s.upsert_message(&a, m).await.unwrap();
    }
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn existing_databases_upgrade_without_changing_mail_or_ownership() {
    let (d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    let before = s
        .upsert_message(
            &a,
            mail(&account.id, "preserved", "INBOX", "2026-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
    drop(a);
    drop(s);
    // Recreate the exact preceding schema with existing user/account/mail data.
    let control = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        d.path().join("control.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::raw_sql("DROP TABLE mail_rules; ALTER TABLE accounts DROP COLUMN connected_at; DELETE FROM _sqlx_migrations WHERE version=7;").execute(&control).await.unwrap();
    control.close().await;
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        d.path()
            .join("accounts")
            .join(format!("{}.sqlite", account.id))
            .display()
    ))
    .await
    .unwrap();
    sqlx::raw_sql("DROP TABLE rule_executions; DROP TABLE rule_inbox_observations; DELETE FROM _sqlx_migrations WHERE version=20;").execute(&db).await.unwrap();
    db.close().await;
    let s = Store::open(d.path(), [42; 32]).await.unwrap();
    let a = s.authorize(&user, &account.id).await.unwrap();
    assert_eq!(s.message(&a, "preserved").await.unwrap(), before);
    assert_eq!(s.accounts(&user).await.unwrap(), vec![account.clone()]);
    assert!(s.rules(&user).await.unwrap().is_empty());
    s.create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    s.upsert_message(&a, mail(&account.id, "after-upgrade", "INBOX", &future()))
        .await
        .unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_some());
}

#[tokio::test]
async fn failed_automatic_moves_are_reported_and_never_automatically_replayed() {
    let (_d, s, user, _, account) = setup().await;
    let a = s.authorize(&user, &account.id).await.unwrap();
    let rule = s
        .create_rule(&user, block("offers@example.org"))
        .await
        .unwrap();
    let m = s
        .upsert_message(&a, mail(&account.id, "failure", "INBOX", &future()))
        .await
        .unwrap();
    assert_eq!(m.revision, s.message(&a, &m.id).await.unwrap().revision);
    let mut op = s.claim_mutation(&a).await.unwrap().unwrap();
    op.status = OperationStatus::Failed;
    s.finish_mutation(&a, op, None).await.unwrap();
    s.upsert_message(&a, mail(&account.id, "failure", "INBOX", &future()))
        .await
        .unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    let activity = s.rule_activity(&a).await.unwrap();
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].rule_id, rule.id);
    assert_eq!(activity[0].failed, 1);
    assert_eq!(activity[0].pending, 0);
}
