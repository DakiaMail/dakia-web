use super::*;
#[tokio::test]
async fn rules_require_auth_csrf_ownership_and_supported_schema() {
    let (_d, app, _, _) = setup().await;
    let body=serde_json::json!({"preset":"block_sender","scope":"all_mailboxes","condition":{"kind":"sender_is","email":"Offers@example.org"},"actions":["trash","suppress_notifications"]}).to_string();
    for (token, csrf, status) in [
        ("", false, StatusCode::UNAUTHORIZED),
        ("alice-token", false, StatusCode::FORBIDDEN),
    ] {
        let r = router(app.clone())
            .oneshot(req("POST", "/api/rules", token, csrf, body.clone()))
            .await
            .unwrap();
        assert_eq!(r.status(), status);
    }
    let r = router(app.clone())
        .oneshot(req("POST", "/api/rules", "alice-token", true, body.clone()))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let rule: MailRule =
        serde_json::from_slice(&to_bytes(r.into_body(), 100000).await.unwrap()).unwrap();
    let r = router(app.clone())
        .oneshot(req("POST", "/api/rules", "alice-token", true, body))
        .await
        .unwrap();
    let again: MailRule =
        serde_json::from_slice(&to_bytes(r.into_body(), 100000).await.unwrap()).unwrap();
    assert_eq!(rule, again);
    let r = router(app.clone())
        .oneshot(req("GET", "/api/rules", "bob-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(&to_bytes(r.into_body(), 10000).await.unwrap()[..], b"[]");
    let path = format!("/api/rules/{}", rule.id);
    let r = router(app.clone())
        .oneshot(req(
            "PATCH",
            &path,
            "bob-token",
            true,
            r#"{"enabled":false}"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = router(app.clone())
        .oneshot(req(
            "PATCH",
            &path,
            "alice-token",
            true,
            r#"{"enabled":false,"condition":{}}"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let r=router(app.clone()).oneshot(req("POST","/api/rules","alice-token",true,r#"{"preset":"block_sender","scope":"all_mailboxes","condition":{"kind":"domain_is","domain":"example.org"},"actions":["trash"]}"#.into())).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let r = router(app)
        .oneshot(req(
            "PATCH",
            &path,
            "alice-token",
            true,
            r#"{"enabled":false}"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let disabled: MailRule =
        serde_json::from_slice(&to_bytes(r.into_body(), 100000).await.unwrap()).unwrap();
    assert!(!disabled.enabled);
}
