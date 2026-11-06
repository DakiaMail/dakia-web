//! Test-only HTTP fixture. Real content authorization, cache and detection; no mail provider or model worker.
use super::tests::fixtures;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use dakia_domain::{Message, MessageContent};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[tokio::test]
#[ignore = "loopback fixture started by playwright.translation.config.ts"]
async fn browser_translation_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let store = dakia_storage::Store::open(dir.path(), [39; 32])
        .await
        .unwrap();
    let user = store.user("fixture", "translation").await.unwrap();
    let mut rows = Vec::new();
    for label in ["Work", "Personal"] {
        let input = serde_json::from_value(serde_json::json!({
            "email": format!("{}@example.invalid", label.to_lowercase()), "display_name": label, "provider": "imap",
            "imap_host":"imap.invalid", "imap_port":993, "imap_tls":"implicit", "smtp_host":"smtp.invalid", "smtp_port":465, "smtp_tls":"implicit", "username":"fixture", "password":"fixture-only"
        })).unwrap();
        let account = store.connect(&user, &input).await.unwrap();
        let a = store.authorize(&user, &account.id).await.unwrap();
        store
            .account_state(&a, dakia_domain::AccountState::Ready)
            .await
            .unwrap();
        for body in fixtures() {
            let message: Message = serde_json::from_value(serde_json::json!({
                "id":body.name, "account_id":a.id(), "thread_id":body.name, "provider_id":body.name,
                "folder_ids":["INBOX"], "sender":{"email":"newsletter@example.invalid","name":"Synthetic newsletter"}, "to":[],
                "subject":body.name, "preview":"Synthetic translation regression", "received_at":"2026-11-01T12:00:00Z",
                "unread":false, "starred":false, "category":null, "category_status":"ready", "revision":1
            })).unwrap();
            let message = store.upsert_message(&a, message).await.unwrap();
            let content: MessageContent = serde_json::from_value(serde_json::json!({
                "message":message, "text":body.text, "html":body.html, "attachments":[], "detected_language":null,
                "message_id":null, "references":[], "cc":[], "reply_to":[]
            })).unwrap();
            store
                .cache_content(&a, &content, 8 * 1024 * 1024)
                .await
                .unwrap();
            rows.push(message);
        }
    }
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(URL_SAFE_NO_PAD.encode(Sha256::digest(b"translation-browser-fixture")))
        .bind(&user)
        .bind("translation-csrf")
        .bind(4102444800_i64)
        .execute(store.control())
        .await
        .unwrap();
    let app = crate::App {
        store,
        mail: crate::mail::MailRuntime::default(),
        config: Arc::new(crate::Config {
            origin: "http://127.0.0.1:4336".into(),
            issuer: "https://id.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture".into(),
            web_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/web/dist")
                .to_string_lossy()
                .into(),
        }),
    };
    let router = crate::router(app).route(
        "/__translation/messages",
        axum::routing::get(move || async move { axum::Json(rows) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4336")
        .await
        .unwrap();
    axum::serve(listener, router).await.unwrap();
}
