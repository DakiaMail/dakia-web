use dakia_domain::ConnectAccount;
use dakia_storage::{Error, Store};

#[tokio::test]
async fn quota_is_account_local_serialized_and_counts_unindexed_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_attachment_budget(dir.path(), [5; 32], 10)
        .await
        .unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let first = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &first.id).await.unwrap();
    let upload = || store.put_attachment(&a, "test.txt".into(), "text/plain".into(), b"123456");
    let (one, two) = tokio::join!(upload(), upload());
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let error = match (one, two) {
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => error,
        _ => panic!("exactly one upload must fit"),
    };
    assert!(matches!(error, Error::QuotaExceeded));
    let kept = store
        .put_attachment(&a, "last.txt".into(), "text/plain".into(), b"1234")
        .await
        .unwrap();
    // Re-pinning an existing ID is idempotent even when the quota is full.
    assert_eq!(
        store
            .pin_attachment(
                &a,
                &kept.id,
                kept.filename.clone(),
                kept.content_type.clone(),
                b"1234"
            )
            .await
            .unwrap(),
        kept
    );
    let second = store.connect(&user, &input).await.unwrap();
    let b = store.authorize(&user, &second.id).await.unwrap();
    let directory = dir.path().join("durable").join(b.id());
    std::fs::create_dir_all(&directory).unwrap();
    let orphan = directory.join(uuid::Uuid::new_v4().to_string());
    std::fs::write(&orphan, b"1234567890").unwrap();
    assert!(matches!(
        store
            .put_attachment(&b, "blocked.txt".into(), "text/plain".into(), b"1")
            .await,
        Err(Error::QuotaExceeded)
    ));
    assert_eq!(store.cleanup_orphan_attachments(&b).await.unwrap(), 0);
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(946684800);
    std::fs::File::open(&orphan)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, b"untouched").unwrap();
    let symlink = directory.join(uuid::Uuid::new_v4().to_string());
    std::os::unix::fs::symlink(&outside, &symlink).unwrap();
    assert_eq!(store.cleanup_orphan_attachments(&b).await.unwrap(), 1);
    assert!(!orphan.exists());
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
    assert!(std::fs::symlink_metadata(&symlink)
        .unwrap()
        .file_type()
        .is_symlink());
    store
        .put_attachment(&b, "allowed.txt".into(), "text/plain".into(), b"1234567890")
        .await
        .unwrap();
    assert!(
        Store::open_with_attachment_budget(dir.path().join("invalid"), [5; 32], 0)
            .await
            .is_err()
    );
}
