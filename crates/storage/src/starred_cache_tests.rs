use super::*;
fn content(a: &AuthorizedAccount, id: &str, starred: bool) -> MessageContent {
    let mut m = message(a, id);
    m.starred = starred;
    serde_json::from_value(serde_json::json!({"message":m,"message_id":null,"references":[],"reply_to":[],"cc":[],"text":"Synthetic body","html":"<p>Synthetic body</p>","attachments":[],"detected_language":null})).unwrap()
}
#[tokio::test]
async fn starred_priority_budget_unstar_oversize_and_account_removal() {
    let (dir, store, user, a, input) = fixture().await;
    let b = store.connect(&user, &input).await.unwrap();
    let b = store.authorize(&user, &b.id).await.unwrap();
    let star = content(&a, "s", true);
    let ordinary = content(&a, "u", false);
    let next = content(&a, "v", false);
    let budget = serde_json::to_vec(&star).unwrap().len() as u64;
    for c in [&star, &ordinary, &next] {
        store.upsert_message(&a, c.message.clone()).await.unwrap();
    }
    store.cache_content(&a, &star, budget).await.unwrap();
    store.cache_content(&a, &ordinary, budget).await.unwrap();
    assert!(store.cached_content(&a, "s").await.unwrap().is_some());
    assert!(store.cached_content(&a, "u").await.unwrap().is_none());
    // Catalogue flags, not the original cached snapshot, determine priority.
    let mut unstar = star.message.clone();
    unstar.starred = false;
    store.upsert_message(&a, unstar).await.unwrap();
    store.cache_content(&a, &next, budget + 2).await.unwrap();
    assert!(store.cached_content(&a, "s").await.unwrap().is_none());
    assert!(store.cached_content(&a, "v").await.unwrap().is_some());
    let db = store.db(&a).await.unwrap();
    let total: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes),0) FROM received_cache")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(total as u64 <= budget + 2);
    let mut huge = star.clone();
    huge.text = "x".repeat(10000);
    store.cache_content(&a, &huge, budget).await.unwrap();
    assert!(store.cached_content(&a, "s").await.unwrap().is_none());
    assert!(store.cache_content(&b, &star, budget).await.is_err());
    assert!(store.starred_cache_rows(&b, "").await.unwrap().is_empty());
    store.remove_account(&a, &input.email).await.unwrap();
    assert!(!dir.path().join("cache").join(a.id()).exists());
    assert!(store.starred_cache_rows(&a, "").await.is_err());
}
