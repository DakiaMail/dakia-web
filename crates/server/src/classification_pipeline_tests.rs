use super::*;

#[tokio::test]
async fn cached_read_backfill_keeps_moving_and_separates_notices_from_correspondence() {
    let (_dir, app, _, a, provider) = controlled().await;
    app.store
        .account_state(&a, AccountState::Ready)
        .await
        .unwrap();
    let rows = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows;
    let mut ids = Vec::new();
    for (i, mut m) in rows.into_iter().enumerate() {
        m.unread = false;
        m.classification = Some(Classification::default());
        m.subject = if i == 0 {
            "Your Example receipt [#12345]"
        } else {
            "Re: Billing: Refund Request"
        }
        .into();
        m = app.store.upsert_message(&a, m).await.unwrap();
        app.store
            .learn_recipients(
                &a,
                "draft:classification-fixture",
                &[m.sender.clone()],
                &m.received_at,
            )
            .await
            .unwrap();
        let body = if i == 0 {
            "Total: EUR 42.00"
        } else {
            "Hi, I can help with your refund."
        };
        let raw = format!(
            "From: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}",
            m.sender.email, m.subject
        );
        let content: MessageContent = serde_json::from_value(serde_json::json!({"message":m,"text":body,"html":"","attachments":[],"message_id":null,"references":[],"cc":[],"reply_to":[]})).unwrap();
        app.store
            .cache_content_with_raw(&a, &content, Some(raw.as_bytes()), 1024 * 1024)
            .await
            .unwrap();
        ids.push(m.id.clone());
    }
    let worker = tokio::spawn(crate::models::run(app.clone()));
    let completed = tokio::time::timeout(Duration::from_millis(1500), async {
        loop {
            let mut ready = true;
            for id in &ids {
                ready &= app
                    .store
                    .message(&a, id)
                    .await
                    .unwrap()
                    .classification
                    .unwrap()
                    .status
                    == CategoryStatus::Ready;
            }
            if ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    worker.abort();
    completed.expect("cached backfill must not idle for two seconds after each message");
    assert_eq!(
        app.store.message(&a, &ids[0]).await.unwrap().inbox_kind(),
        InboxKind::Automated
    );
    assert_eq!(
        app.store.message(&a, &ids[1]).await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
    for id in ids {
        assert!(!app.store.message(&a, &id).await.unwrap().unread);
    }
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 0);
}
