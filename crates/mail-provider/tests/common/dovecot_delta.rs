use super::*;
struct Known(bool);
#[async_trait::async_trait]
impl MetadataLookup for Known {
    async fn identity_known(&self, _: &Identity) -> Result<bool> {
        Ok(self.0)
    }
    async fn reference_known(&self, _: &MessageRef) -> Result<bool> {
        Ok(self.0)
    }
}
#[tokio::test]
#[ignore = "requires disposable Dovecot fixture; see fixtures/README.md"]
async fn dovecot_condstore_qresync_changes_and_expunges_preserve_unchanged_members() {
    let (config, cert) = settings(Security::Tls);
    let mut setup = fixture_session().await;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = format!("Delta-source-{run}");
    let dest = format!("Delta-dest-{run}");
    setup.create(&source).await.unwrap();
    setup.create(&dest).await.unwrap();
    for n in 0..2 {
        let raw = format!("From: fixture@example.test\r\nSubject: delta {n}\r\nMessage-ID: <delta-{run}-{n}@example.test>\r\n\r\nSynthetic only\r\n");
        setup
            .append(&source, None, None, raw.as_bytes())
            .await
            .unwrap();
    }
    let provider = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_pem(&cert).unwrap())
        .unwrap();
    let first = provider
        .reconcile(&source, None, 200, &Known(false), None)
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 2);
    let checkpoint = first.checkpoint.expect("Dovecot supports CONDSTORE");
    assert!(provider.snapshot_unchanged(&checkpoint).await.unwrap());
    let reference = match &first.rows[0] {
        MetadataChange::New(m) => m.reference.clone(),
        _ => panic!("initial metadata requires headers"),
    };
    provider
        .mutate(&reference, Mutation::Seen(true))
        .await
        .unwrap();
    assert!(!provider.snapshot_unchanged(&checkpoint).await.unwrap());
    assert!(provider
        .verify_mutation(
            &reference,
            &Identity::Imap(reference.clone()),
            &Mutation::Seen(true)
        )
        .await
        .unwrap()
        .is_some());
    let delta = provider
        .reconcile(&source, None, 200, &Known(true), Some(&checkpoint))
        .await
        .unwrap();
    assert_eq!(delta.rows.len(), 2);
    assert_eq!(
        delta
            .rows
            .iter()
            .filter(|r| matches!(r, MetadataChange::Unchanged(_)))
            .count(),
        1
    );
    assert!(delta
        .rows
        .iter()
        .any(|r| matches!(r, MetadataChange::Mutable(m) if m.reference == reference && m.seen)));
    assert!(provider
        .snapshot_unchanged(delta.checkpoint.as_ref().unwrap())
        .await
        .unwrap());
    provider
        .mutate(&reference, Mutation::Move(dest))
        .await
        .unwrap();
    assert!(!provider
        .snapshot_unchanged(delta.checkpoint.as_ref().unwrap())
        .await
        .unwrap());
    let after = provider
        .reconcile(&source, None, 200, &Known(true), delta.checkpoint.as_ref())
        .await
        .unwrap();
    assert_eq!(
        after.rows.len(),
        1,
        "expunged UID is absent from full membership enumeration"
    );
    assert!(after.next.is_none() && after.full_coverage);
    let latest = after.checkpoint.as_ref().unwrap();
    assert!(provider.snapshot_unchanged(latest).await.unwrap());
    setup
        .append(
            &source,
            None,
            None,
            b"From: fixture@example.test\r\nSubject: arrival\r\n\r\nnew",
        )
        .await
        .unwrap();
    assert!(!provider.snapshot_unchanged(latest).await.unwrap());
    setup.logout().await.unwrap();
}
