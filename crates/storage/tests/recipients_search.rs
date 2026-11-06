use dakia_domain::*;
use dakia_storage::{Error, Store};
#[tokio::test]
async fn opaque_search_cursor_is_owner_bound_expiring_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [1; 32]).await.unwrap();
    let owner = store.user("fixture", "owner").await.unwrap();
    let other = store.user("fixture", "other").await.unwrap();
    // This exceeded the old URL token's hard 10,000-seen-result cutoff.
    let document = serde_json::to_string(
        &(0..20000)
            .map(|i| format!("account:message-{i}"))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let token = store.save_search_cursor(&owner, &document).await.unwrap();
    assert_eq!(token.len(), 36);
    assert!(matches!(
        store.load_search_cursor(&other, &token).await,
        Err(Error::CursorExpired)
    ));
    assert!(store
        .load_search_cursor(&owner, "not-a-token")
        .await
        .is_err());
    let restarted = Store::open(dir.path(), [1; 32]).await.unwrap();
    assert_eq!(
        restarted.load_search_cursor(&owner, &token).await.unwrap(),
        document
    );
    sqlx::query("UPDATE search_cursors SET expires_at=0 WHERE id=?")
        .bind(&token)
        .execute(store.control())
        .await
        .unwrap();
    assert!(matches!(
        restarted.load_search_cursor(&owner, &token).await,
        Err(Error::CursorExpired)
    ));
    store.save_search_cursor(&owner, "{}").await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_cursors WHERE id=?")
        .bind(&token)
        .fetch_one(store.control())
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn recipient_evidence_ranking_is_account_scoped_and_rescan_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [1; 32]).await.unwrap();
    let owner = store.user("fixture", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&owner, &input).await.unwrap();
    let a = store.authorize(&owner, &account.id).await.unwrap();
    let friend = Address {
        email: "friend@example.invalid".into(),
        name: Some("Friend".into()),
    };
    let recent = Address {
        email: "recent@example.invalid".into(),
        name: None,
    };
    for _ in 0..5 {
        store
            .learn_recipients(
                &a,
                "sent:one",
                std::slice::from_ref(&recent),
                "2026-10-28T18:00:00Z",
            )
            .await
            .unwrap();
    }
    for source in ["sent:two", "draft:three"] {
        store
            .learn_recipients(
                &a,
                source,
                std::slice::from_ref(&friend),
                "2026-10-27T18:00:00Z",
            )
            .await
            .unwrap();
    }
    assert_eq!(
        store.recipient_suggestions(&a, "").await.unwrap(),
        vec![friend.clone(), recent]
    );
    assert_eq!(
        store.recipient_suggestions(&a, "FRIEND").await.unwrap(),
        vec![friend]
    );
    assert!(store
        .known_recipient(&a, "FRIEND@example.invalid")
        .await
        .unwrap());
    assert!(!store
        .known_recipient(&a, "stranger@example.invalid")
        .await
        .unwrap());
}
