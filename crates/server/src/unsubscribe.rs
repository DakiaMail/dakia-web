//! Sender-provided list actions. Inspection never contacts an unsubscribe endpoint.
use crate::*;
use dakia_storage::AuthorizedAccount;
use mail_auth::{AuthenticatedMessage, DkimResult, MessageAuthenticator};
use serde::Serialize;
use std::{net::SocketAddr, time::Duration};
use tokio::sync::Semaphore;
use url::Url;

static LANES: Semaphore = Semaphore::const_new(4);
// Only the test binary can replace this exact synthetic transport destination.
// Production always resolves, validates public addresses and pins DNS below.
#[cfg(test)]
pub(crate) static BROWSER_HTTP: std::sync::OnceLock<(reqwest::Client, Url)> =
    std::sync::OnceLock::new();
#[cfg(test)]
#[path = "unsubscribe_dkim_tests.rs"]
mod dkim_tests;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Web {
        url: String,
    },
    OneClick {
        url: String,
    },
    Mailto {
        to: String,
        subject: String,
        body: String,
    },
}
fn rejected() -> ApiFailure {
    ApiFailure(StatusCode::BAD_REQUEST, "unsubscribe_unavailable")
}
fn safe_web(value: &str) -> Option<Url> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    let host = match url.host()? {
        url::Host::Domain(host) => {
            host.contains('.')
                && ![
                    "localhost",
                    "local",
                    "internal",
                    "home",
                    "lan",
                    "test",
                    "invalid",
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        }
        url::Host::Ipv4(ip) => images::public(ip.into()),
        url::Host::Ipv6(ip) => images::public(ip.into()),
    };
    (matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none()
        && host
        && url.port_or_known_default() == Some(if url.scheme() == "https" { 443 } else { 80 }))
    .then_some(url)
}
fn mailto(url: &Url) -> Option<Action> {
    if url.scheme() != "mailto" || url.host().is_some() || url.fragment().is_some() {
        return None;
    }
    let decode = |v: &str| {
        let bytes = v.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'%'
                && (i + 2 >= bytes.len()
                    || !bytes[i + 1].is_ascii_hexdigit()
                    || !bytes[i + 2].is_ascii_hexdigit())
            {
                return None;
            }
        }
        percent_encoding::percent_decode_str(v)
            .decode_utf8()
            .ok()
            .map(|s| s.into_owned())
    };
    let mut to = decode(url.path())?;
    let mut subject = String::new();
    let mut body = String::new();
    let mut seen = std::collections::HashSet::new();
    for field in url
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|s| !s.is_empty())
    {
        let (name, value) = field.split_once('=').unwrap_or((field, ""));
        let name = decode(name)?.to_ascii_lowercase();
        let value = decode(value)?;
        if !seen.insert(name.clone()) {
            return None;
        }
        match name.as_str() {
            "to" if to.is_empty() => to = value,
            "subject" => subject = value,
            "body" => body = value,
            // Do not silently discard cc/bcc or ambiguous recipient instructions.
            _ => return None,
        }
    }
    let (local, domain) = to.rsplit_once('@')?;
    if to.len() > 320
        || to.matches('@').count() != 1
        || local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || !local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&b))
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        || subject.len() > 998
        || subject.chars().any(char::is_control)
        || body.len() > 65536
        || body.contains('\0')
    {
        return None;
    }
    Some(Action::Mailto { to, subject, body })
}
fn headers(raw: &[u8], name: &str) -> Vec<String> {
    let Some(mail) = mail_parser::MessageParser::default().parse(raw) else {
        return vec![];
    };
    mail.headers_raw()
        .filter(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.replace(['\r', '\n'], " ").trim().to_owned())
        .collect()
}
fn urls(values: &[String]) -> Vec<Url> {
    values
        .iter()
        .flat_map(|value| {
            value.split('<').skip(1).filter_map(|rest| {
                rest.split_once('>')
                    .and_then(|(url, _)| Url::parse(url.trim()).ok())
            })
        })
        .collect()
}
fn select(raw: &[u8], verified: bool) -> Option<Action> {
    let list = headers(raw, "List-Unsubscribe");
    let post = headers(raw, "List-Unsubscribe-Post");
    let urls = urls(&list);
    if verified
        && list.len() == 1
        && post.len() == 1
        && post[0].trim() == "List-Unsubscribe=One-Click"
    {
        if let Some(url) = urls
            .iter()
            .filter_map(|url| safe_web(url.as_str()))
            .find(|u| u.scheme() == "https" && u.fragment().is_none())
        {
            return Some(Action::OneClick { url: url.into() });
        }
    }
    // Prefer a supported mail request to an arbitrary provider webpage.
    for url in &urls {
        if let Some(action) = mailto(url) {
            return Some(action);
        }
    }
    for url in urls {
        if let Some(url) = safe_web(url.as_str()) {
            return Some(Action::Web { url: url.into() });
        }
        if let Some(action) = mailto(&url) {
            return Some(action);
        }
    }
    None
}
fn covered(results: &[mail_auth::DkimOutput<'_>]) -> bool {
    results.iter().any(|r| {
        matches!(r.result(), DkimResult::Pass)
            && r.signature().is_some_and(|s| {
                ["List-Unsubscribe", "List-Unsubscribe-Post"]
                    .iter()
                    .all(|name| s.h.iter().any(|h| h.eq_ignore_ascii_case(name)))
            })
    })
}
async fn inspect(raw: &[u8]) -> Option<Action> {
    // Never trust Authentication-Results supplied by the sender. As on desktop,
    // RFC 8058 requires a passing signature covering BOTH unique list headers.
    let needs_verification = headers(raw, "List-Unsubscribe").len() == 1
        && headers(raw, "List-Unsubscribe-Post") == ["List-Unsubscribe=One-Click"];
    let verified = if needs_verification {
        let check = async {
            let auth = MessageAuthenticator::new_system_conf().ok()?;
            let message = AuthenticatedMessage::parse(raw)?;
            Some(covered(&auth.verify_dkim(&message).await))
        };
        tokio::time::timeout(Duration::from_secs(5), check)
            .await
            .ok()
            .flatten()
            .unwrap_or(false)
    } else {
        false
    };
    select(raw, verified)
}
pub(crate) async fn remember(
    store: &Store,
    a: &AuthorizedAccount,
    id: &str,
    raw: &[u8],
) -> Result<Option<Action>> {
    let action = inspect(raw).await;
    store
        .put_provider_document(
            a,
            &format!("unsubscribe-v1:{id}"),
            &serde_json::to_string(&action).map_err(dakia_storage::Error::from)?,
        )
        .await?;
    Ok(action)
}
async fn action(app: &App, a: &AuthorizedAccount, id: &str) -> Result<Option<Action>> {
    app.store.message(a, id).await?;
    if let Some(doc) = app
        .store
        .provider_document(a, &format!("unsubscribe-v1:{id}"))
        .await?
    {
        return serde_json::from_str(&doc)
            .map_err(dakia_storage::Error::from)
            .map_err(Into::into);
    }
    let raw = app.mail.raw(&app.store, a, id).await?;
    remember(&app.store, a, id, &raw).await
}
pub(crate) async fn info(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let mut value =
        serde_json::to_value(action(&app, &a, &id).await?).map_err(dakia_storage::Error::from)?;
    if let Some(object) = value.as_object_mut() {
        let status = attempt_status(&app.store, &a, &id).await?;
        object.insert("attempt_status".into(), status.into());
    }
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Confirmation {
    expected_url: Option<String>,
    expected_action: Option<Action>,
}
async fn attempt_status(store: &Store, a: &AuthorizedAccount, id: &str) -> Result<Option<String>> {
    let record = store
        .provider_document(a, &format!("unsubscribe-attempt:{id}"))
        .await?;
    if let Some(id) = record
        .as_deref()
        .and_then(|r| r.strip_prefix("submission:"))
    {
        let s = store.submission(a, id).await?;
        return Ok(Some(
            match s.state {
                SubmissionState::Accepted => "request_accepted",
                SubmissionState::Failed | SubmissionState::Cancelled => "not_sent",
                SubmissionState::DeliveryUnknown => "result_unknown",
                _ => "attempted",
            }
            .into(),
        ));
    }
    Ok(record)
}
pub(crate) async fn execute(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
    Json(input): Json<Confirmation>,
) -> Result<Response> {
    let a = app.store.authorize(&user.user, &account).await?;
    let Some(current) = action(&app, &a, &id).await? else {
        return Err(rejected());
    };
    if input.expected_action.as_ref() != Some(&current)
        && !matches!((&current, &input.expected_url), (Action::OneClick { url }, Some(expected)) if url == expected && input.expected_action.is_none())
    {
        return Err(rejected());
    }
    if let Action::Mailto { to, subject, body } = &current {
        // Re-validate even persisted capabilities. No client supplied recipients,
        // original body, attachments, cc/bcc, reply headers or HTML are forwarded.
        let encoded = format!(
            "mailto:{}?subject={}&body={}",
            percent_encoding::utf8_percent_encode(to, percent_encoding::NON_ALPHANUMERIC),
            percent_encoding::utf8_percent_encode(subject, percent_encoding::NON_ALPHANUMERIC),
            percent_encoding::utf8_percent_encode(body, percent_encoding::NON_ALPHANUMERIC)
        );
        if mailto(&Url::parse(&encoded).map_err(|_| rejected())?) != Some(current.clone()) {
            return Err(rejected());
        }
        let draft = Draft {
            id: format!("unsubscribe:{id}"),
            account_id: account.clone(),
            revision: 1,
            to: vec![Address {
                email: to.clone(),
                name: None,
            }],
            cc: vec![],
            bcc: vec![],
            subject: subject.clone(),
            text: body.clone(),
            html: String::new(),
            in_reply_to: None,
            references: vec![],
            attachments: vec![],
            saved_at: String::new(),
        };
        app.store
            .enqueue_unsubscribe(&a, &format!("unsubscribe-attempt:{id}"), draft)
            .await?;
        return match attempt_status(&app.store, &a, &id).await?.as_deref() {
            Some("request_accepted") => Ok(StatusCode::NO_CONTENT.into_response()),
            Some("attempted") => Ok((
                StatusCode::ACCEPTED,
                Json(serde_json::json!({"queued":true})),
            )
                .into_response()),
            _ => Err(ApiFailure(
                StatusCode::CONFLICT,
                "unsubscribe_already_attempted",
            )),
        };
    }
    let Action::OneClick { url } = current else {
        return Err(rejected());
    };
    // Claim before the side effect. Never automatically repeat an uncertain POST,
    // including after restart or a second tab. There is no force/retry endpoint.
    let operation = format!("unsubscribe-attempt:{id}");
    let _lane = LANES
        .try_acquire()
        .map_err(|_| ApiFailure(StatusCode::TOO_MANY_REQUESTS, "unsubscribe_busy"))?;
    // Store's unique idempotency record makes this atomic across requests/processes.
    if !app.store.claim_unsubscribe(&a, &operation).await? {
        if app
            .store
            .provider_document(&a, &operation)
            .await?
            .as_deref()
            == Some("request_accepted")
        {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        return Err(ApiFailure(
            StatusCode::CONFLICT,
            "unsubscribe_already_attempted",
        ));
    }
    let result = post(&url).await;
    // Persist the transport result, not a claim that the subscription was removed.
    // An interrupted process leaves 'attempted', which is deliberately uncertain.
    let outcome = match &result {
        Ok(()) => "request_accepted",
        Err(ApiFailure(_, "unsubscribe_not_confirmed")) => "request_rejected",
        Err(ApiFailure(_, "unsubscribe_unavailable")) => "not_sent",
        Err(_) => "result_unknown",
    };
    app.store
        .put_provider_document(&a, &operation, outcome)
        .await?;
    result?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
async fn post(value: &str) -> Result<()> {
    let url = safe_web(value)
        .filter(|u| u.scheme() == "https" && u.fragment().is_none())
        .ok_or_else(rejected)?;
    #[cfg(test)]
    if value == "https://newsletter.example.org/browser-synthetic" {
        if let Some((client, destination)) = BROWSER_HTTP.get() {
            return send(client, destination.clone()).await;
        }
    }
    let host = url.host_str().ok_or_else(rejected)?;
    let addresses: Vec<SocketAddr> =
        tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 443)))
            .await
            .map_err(|_| rejected())?
            .map_err(|_| rejected())?
            .collect();
    if addresses.is_empty()
        || addresses.len() > 32
        || addresses.iter().any(|a| !images::public(a.ip()))
    {
        return Err(rejected());
    }
    let client = reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| rejected())?;
    send(&client, url).await
}
async fn send(client: &reqwest::Client, url: Url) -> Result<()> {
    // No cookies, credentials, Referer, redirects, retries or response-body download.
    let response = client
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .send()
        .await
        .map_err(|error| {
            ApiFailure(
                StatusCode::BAD_GATEWAY,
                if error.is_connect() {
                    "unsubscribe_unavailable"
                } else {
                    "unsubscribe_result_unknown"
                },
            )
        })?;
    if !response.status().is_success() {
        return Err(ApiFailure(
            StatusCode::BAD_GATEWAY,
            "unsubscribe_not_confirmed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_prefers_verified_post_then_supported_mail_over_web() {
        let raw = b"List-Unsubscribe: <https://example.org/u>, <mailto:leave@example.org?subject=Unsubscribe>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\nHi";
        assert!(matches!(select(raw, false), Some(Action::Mailto { .. })));
        assert!(matches!(select(raw, true), Some(Action::OneClick { .. })));
        let duplicate = [
            b"List-Unsubscribe: <https://example.org/other>\r\n".as_slice(),
            raw,
        ]
        .concat();
        assert!(matches!(
            select(&duplicate, true),
            Some(Action::Mailto { .. })
        ));
        assert_eq!(
            select(
                b"Subject: No list\r\n\r\nunsubscribe https://example.org",
                true
            ),
            None
        );
    }
    #[test]
    fn unsafe_and_ambiguous_actions_are_not_supported() {
        for value in [
            "javascript:alert(1)",
            "https://127.0.0.1/u",
            "https://[::1]/",
            "https://10.0.0.1/",
            "https://localhost/",
            "https://foo.local/",
            "https://user:pass@example.org/u",
            "https://example.org:8443/u",
            "mailto:a@example.org,b@example.org",
            "mailto:a@example.org?subject=x%0d%0aBcc:other@example.org",
            "mailto:a@example.org?bcc=other@example.org",
            "mailto:a@example.org?to=other@example.org",
            "mailto:a@example.org?attach=private.txt",
            "mailto:a@example.org?subject=one&SUBJECT=two",
            "mailto:a%0d%0ab@example.org",
            "mailto:a@example.org?subject=%GG",
            "mailto:a@example.org?body=%FF",
            "mailto:a@example.org?body=%00",
            "mailto:a@example.org?from=other@example.org",
            "mailto:a@example.org?subject=x%0ABcc%3Ab@example.org",
            "mailto:a..b@example.org",
            "mailto:a@-example.org",
            "mailto:a@example.org,b@example.org?body=x",
        ] {
            let raw = format!("List-Unsubscribe: <{value}>\r\n\r\nHi");
            assert_eq!(select(raw.as_bytes(), true), None, "{value}");
        }
        assert_eq!(select(b"List-Unsubscribe: <mailto:leave+token@example.org?subject=Leave%2Bnow&body=Stop%0D%0Aplease>\r\n\r\nHi", false), Some(Action::Mailto { to: "leave+token@example.org".into(), subject: "Leave+now".into(), body: "Stop\r\nplease".into() }));
    }
    #[tokio::test]
    async fn forged_authentication_results_never_enable_one_click() {
        let raw = b"Authentication-Results: mx.example.org; dkim=pass\r\nList-Unsubscribe: <https://example.org/u>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\nHi";
        assert!(matches!(inspect(raw).await, Some(Action::Web { .. })));
    }
    #[tokio::test]
    async fn local_transport_checks_exact_post_and_errors_without_external_requests() {
        use axum::routing::post;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let slow_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow_count = slow_requests.clone();
        let app = Router::new()
            .route(
                "/ok",
                post(|headers: HeaderMap, body: String| async move {
                    assert_eq!(headers["content-type"], "application/x-www-form-urlencoded");
                    assert!(!headers.contains_key("cookie"));
                    assert!(!headers.contains_key("referer"));
                    assert_eq!(body, "List-Unsubscribe=One-Click");
                    StatusCode::NO_CONTENT
                }),
            )
            .route("/error", post(|| async { StatusCode::BAD_REQUEST }))
            .route(
                "/server-error",
                post(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
            )
            .route("/accepted", post(|| async { StatusCode::ACCEPTED }))
            .route(
                "/slow",
                post(move || {
                    let count = slow_count.clone();
                    async move {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        StatusCode::NO_CONTENT
                    }
                }),
            )
            .route(
                "/redirect",
                post(|| async { (StatusCode::FOUND, [("location", "/ok")]) }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .retry(reqwest::retry::never())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert!(send(
            &client,
            Url::parse(&format!("http://{address}/ok")).unwrap()
        )
        .await
        .is_ok());
        assert!(send(
            &client,
            Url::parse(&format!("http://{address}/accepted")).unwrap()
        )
        .await
        .is_ok());
        for path in ["error", "server-error", "redirect"] {
            assert!(send(
                &client,
                Url::parse(&format!("http://{address}/{path}")).unwrap()
            )
            .await
            .is_err());
        }
        assert!(super::post(&format!("https://{address}/ok")).await.is_err());
        let error = send(
            &client,
            Url::parse(&format!("http://{address}/slow")).unwrap(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.1, "unsubscribe_result_unknown");
        assert_eq!(slow_requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        server.abort();
    }
}
