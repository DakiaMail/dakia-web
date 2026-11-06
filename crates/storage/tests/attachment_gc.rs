use dakia_domain::*;
use dakia_storage::Store;
#[tokio::test]
async fn abandoned_uploads_expire_without_losing_drafts_or_submission_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [1; 32]).await.unwrap();
    let owner = store.user("fixture", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&owner, &input).await.unwrap();
    let a = store.authorize(&owner, &account.id).await.unwrap();
    let mut parts = vec![];
    for _ in 0..4 {
        parts.push(
            store
                .put_attachment(&a, "test.txt".into(), "text/plain".into(), b"synthetic")
                .await
                .unwrap(),
        );
    }
    let mut draft: Draft = serde_json::from_value(serde_json::json!({"id":"draft","account_id":a.id(),"revision":0,"to":[{"email":"recipient@example.invalid","name":null}],"cc":[],"bcc":[],"subject":"Synthetic only","text":"Snapshot","html":"<p>Snapshot</p>","in_reply_to":null,"references":[],"attachments":[parts[0]],"saved_at":""})).unwrap();
    draft = store.save_draft(&a, draft).await.unwrap();
    let submission = store
        .submit(
            &a,
            &SubmitRequest {
                operation_id: "gc-test".into(),
                draft_id: draft.id.clone(),
                expected_revision: draft.revision,
                due_at: None,
                timezone: "UTC".into(),
            },
        )
        .await
        .unwrap();
    draft.attachments = vec![parts[1].clone()];
    store.save_draft(&a, draft).await.unwrap();
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path()
            .join("accounts")
            .join(format!("{}.sqlite", a.id()))
            .display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE blobs SET last_access='2000-01-01T00:00:00.000Z' WHERE id<>?")
        .bind(&parts[3].id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(store.cleanup_orphan_attachments(&a).await.unwrap(), 1);
    assert!(store.attachment(&a, &parts[2].id).await.is_err());
    for index in [0, 1, 3] {
        assert_eq!(
            store.attachment(&a, &parts[index].id).await.unwrap().1,
            b"synthetic"
        );
    }
    assert_eq!(
        store
            .submission_snapshot(&a, &submission.id)
            .await
            .unwrap()
            .attachments,
        vec![parts[0].clone()]
    );
    assert_eq!(store.cleanup_orphan_attachments(&a).await.unwrap(), 0);
    store.backup(dir.path().join("backup")).await.unwrap();
    let restored = Store::restore(
        dir.path().join("backup"),
        dir.path().join("restored"),
        [1; 32],
    )
    .await
    .unwrap();
    let restored_account = restored.authorize(&owner, a.id()).await.unwrap();
    assert_eq!(
        restored
            .attachment(&restored_account, &parts[0].id)
            .await
            .unwrap()
            .1,
        b"synthetic"
    );
    assert!(!restored.jobs_enabled().await.unwrap());
}
