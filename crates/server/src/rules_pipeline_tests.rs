use super::*;
use dakia_domain::{CreateRule, RuleAction, RuleCondition, RulePreset, RuleScope};

#[tokio::test]
async fn blocked_sender_arrivals_use_real_reconciliation_and_verified_trash_queue() {
    for head in [false, true] {
        let (_dir, app, user, a, provider) = controlled().await;
        app.mail.probe(&app.store, &a).await.unwrap();
        app.mail
            .snapshot_page(&app.store, &a, "INBOX", None)
            .await
            .unwrap();
        let old = app.store.inbox_rows(&a, None, None).await.unwrap();
        app.store
            .create_rule(
                &user,
                CreateRule {
                    preset: RulePreset::BlockSender,
                    scope: RuleScope::AllMailboxes,
                    condition: RuleCondition::SenderIs {
                        email: "FIXTURE@example.invalid".into(),
                    },
                    actions: vec![RuleAction::Trash, RuleAction::SuppressNotifications],
                },
            )
            .await
            .unwrap();
        let mut new = provider.page.lock().await.rows[0].clone();
        new.reference.uid = 99;
        new.identity = p::Identity::Imap(new.reference.clone());
        new.headers=b"From: fixture@example.invalid\r\nSubject: Blocked arrival\r\nMessage-ID: <blocked-arrival@example.invalid>\r\n\r\n".to_vec();
        new.internal_date = (chrono::Utc::now() + chrono::Duration::seconds(10))
            .format("%d-%b-%Y %H:%M:%S %z")
            .to_string();
        provider.page.lock().await.rows.push(new.clone());
        new.reference.uid = 100;
        new.identity = p::Identity::Imap(new.reference.clone());
        new.headers = b"From: fixture@example.invalid\r\nSubject: Second blocked arrival\r\nMessage-ID: <second-blocked@example.invalid>\r\n\r\n".to_vec();
        provider.page.lock().await.rows.push(new);
        if head {
            app.mail.poll_head(&app.store, &a, "INBOX").await.unwrap();
        } else {
            app.mail
                .snapshot_page(&app.store, &a, "INBOX", None)
                .await
                .unwrap();
        }
        let executions = app.store.rule_executions(&a).await.unwrap();
        assert_eq!(executions.len(), 2);
        assert_eq!(executions[0].state, "pending");
        assert!(provider.mutations.lock().await.is_empty());
        crate::mutations::drain(&app, &a).await.unwrap();
        assert_eq!(provider.mutations.lock().await.len(), 2);
        let message = app
            .store
            .message(&a, &executions[0].message_id)
            .await
            .unwrap();
        assert_eq!(message.folder_ids, vec!["Trash"]);
        // Refreshing after the provider applied the move must not requeue it.
        app.mail
            .snapshot_page(&app.store, &a, "INBOX", None)
            .await
            .unwrap();
        crate::mutations::drain(&app, &a).await.unwrap();
        assert_eq!(provider.mutations.lock().await.len(), 2);
        for message in old {
            assert!(app
                .store
                .message(&a, &message.id)
                .await
                .unwrap()
                .folder_ids
                .contains(&"INBOX".into()));
        }
    }
}
