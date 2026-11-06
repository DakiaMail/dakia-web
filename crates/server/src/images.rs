//! Explicit authenticated remote-image loading. Every redirect is independently pinned.
use crate::*;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::sync::Semaphore;
static IMAGE_LANES: Semaphore = Semaphore::const_new(4);
const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

fn redirect_blocked() -> ApiFailure {
    ApiFailure(StatusCode::BAD_REQUEST, "remote_image_redirect_blocked")
}

fn blocked() -> ApiFailure {
    ApiFailure(StatusCode::BAD_REQUEST, "remote_image_rejected")
}
fn unavailable() -> ApiFailure {
    ApiFailure(StatusCode::BAD_GATEWAY, "remote_image_unavailable")
}
fn too_large() -> ApiFailure {
    ApiFailure(StatusCode::PAYLOAD_TOO_LARGE, "remote_image_too_large")
}
fn unsupported() -> ApiFailure {
    ApiFailure(StatusCode::BAD_REQUEST, "remote_image_unsupported_content")
}
fn target(value: &str) -> Result<url::Url> {
    if value.len() > 4096 {
        return Err(blocked());
    }
    let url = url::Url::parse(value).map_err(|_| blocked())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !matches!(url.host(), Some(url::Host::Domain(_)))
    {
        return Err(blocked());
    }
    Ok(url)
}
pub(crate) fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192
                    && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)))
                || (a == 198 && ((b == 18 || b == 19) || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let [a, b, _, _, _, _, _, _] = ip.segments();
            (a & 0xe000) == 0x2000
                && !(a == 0x2001 && b < 0x0200)
                && a != 0x2002
                && !(a == 0x2001 && b == 0x0db8)
                && !(a == 0x3fff && b < 0x1000)
        }
    }
}
fn image_type(content_type: &str, bytes: &[u8]) -> Option<&'static str> {
    let content_type = content_type.split(';').next()?.trim().to_ascii_lowercase();
    match content_type.as_str() {
        "image/png" | "image/x-png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Some("image/png"),
        "image/jpeg" | "image/jpg" | "image/pjpeg" if bytes.starts_with(b"\xff\xd8\xff") => {
            Some("image/jpeg")
        }
        "image/gif" if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") => {
            Some("image/gif")
        }
        "image/webp" if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") => {
            Some("image/webp")
        }
        _ => None,
    }
}
// Test real HTTP responses without relaxing the production target/DNS/TLS checks.
async fn read_image(mut response: reqwest::Response) -> Result<String> {
    if response.status().is_redirection() {
        return Err(redirect_blocked());
    }
    if !response.status().is_success() {
        return Err(unavailable());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err(too_large());
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unsupported)?
        .to_owned();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    let kind = image_type(&content_type, &bytes).ok_or_else(unsupported)?;
    Ok(format!(
        "data:{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
// Only DNS and transport are replaceable in deterministic tests. URL/address
// validation, pinning, client/request construction and the coordinator stay real.
#[async_trait::async_trait]
trait ImageNetwork: Sync {
    async fn resolve(&self, host: &str) -> Result<Vec<SocketAddr>>;
    async fn send(
        &self,
        client: reqwest::Client,
        request: reqwest::Request,
    ) -> Result<reqwest::Response>;
}
struct PublicNetwork;
#[async_trait::async_trait]
impl ImageNetwork for PublicNetwork {
    async fn resolve(&self, host: &str) -> Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host((host, 443))
            .await
            .map_err(|_| unavailable())?
            .collect())
    }
    async fn send(
        &self,
        client: reqwest::Client,
        request: reqwest::Request,
    ) -> Result<reqwest::Response> {
        client.execute(request).await.map_err(|_| unavailable())
    }
}
async fn fetch_chain(value: &str, network: &impl ImageNetwork) -> Result<String> {
    let mut url = target(value)?;
    let mut visited = std::collections::HashSet::new();
    for hop in 0..=MAX_REDIRECTS {
        // Fragments never reach HTTP; ignoring them also catches fragment loops.
        url.set_fragment(None);
        if !visited.insert(url.as_str().to_owned()) {
            return Err(redirect_blocked());
        }
        let host = url.host_str().ok_or_else(blocked)?;
        let addresses = network.resolve(host).await?;
        if addresses.is_empty()
            || addresses.len() > 32
            || addresses.iter().any(|a| a.port() != 443 || !public(a.ip()))
        {
            return Err(if hop == 0 {
                blocked()
            } else {
                redirect_blocked()
            });
        }
        // New client per hop: no connection reuse across DNS validations, cookie
        // store, automatic redirects, proxy, auth, referer or forwarded headers.
        // TLS still verifies the original hostname against normal trusted roots.
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|_| blocked())?;
        let request = client
            .get(url.clone())
            // An honest, fixed identifier avoids rejecting anonymous clients at
            // static CDNs. Never send browser, account or recipient metadata.
            .header(header::USER_AGENT, "Dakia/0.1 (remote email image proxy)")
            .header("accept", "image/png,image/jpeg,image/gif,image/webp")
            .build()
            .map_err(|_| blocked())?;
        let response = network.send(client, request).await?;
        if !response.status().is_redirection() {
            return read_image(response).await;
        }
        if hop == MAX_REDIRECTS
            || !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308)
        {
            return Err(redirect_blocked());
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 4096
                    && !v.bytes().any(|b| b.is_ascii_control() || b == b' ')
            })
            .ok_or_else(redirect_blocked)?;
        let next = url.join(location).map_err(|_| redirect_blocked())?;
        url = target(next.as_str()).map_err(|_| redirect_blocked())?;
        // Drop redirect bodies without reading them. The deadline below includes
        // all DNS, connections, redirects and the final bounded body stream.
    }
    Err(redirect_blocked())
}
async fn fetch_with_network(
    value: &str,
    network: &impl ImageNetwork,
    budget: Duration,
) -> Result<String> {
    tokio::time::timeout(budget, fetch_chain(value, network))
        .await
        .map_err(|_| unavailable())?
}
async fn fetch(value: &str) -> Result<String> {
    let _lane = IMAGE_LANES
        .try_acquire()
        .map_err(|_| ApiFailure(StatusCode::TOO_MANY_REQUESTS, "image_loading_busy"))?;
    fetch_with_network(value, &PublicNetwork, TOTAL_TIMEOUT).await
}
pub(crate) async fn load(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
    Json(input): Json<RemoteImageRequest>,
) -> Result<Json<String>> {
    let a = app.store.authorize(&user.user, &account).await?;
    app.store.message(&a, &id).await?;
    Ok(Json(fetch(&input.url).await?))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct ScriptNetwork {
        replies: Mutex<VecDeque<(String, Vec<u8>)>>,
        dns: Mutex<VecDeque<Vec<SocketAddr>>>,
        hosts: Mutex<Vec<String>>,
        sent: Mutex<Vec<String>>,
        delay: Duration,
    }
    impl ScriptNetwork {
        fn new(replies: Vec<(String, Vec<u8>)>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                dns: Mutex::new(VecDeque::new()),
                hosts: Mutex::new(vec![]),
                sent: Mutex::new(vec![]),
                delay: Duration::ZERO,
            }
        }
    }
    #[async_trait::async_trait]
    impl ImageNetwork for ScriptNetwork {
        async fn resolve(&self, host: &str) -> Result<Vec<SocketAddr>> {
            self.hosts.lock().unwrap().push(host.into());
            Ok(self
                .dns
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| vec!["8.8.8.8:443".parse().unwrap()]))
        }
        async fn send(
            &self,
            _client: reqwest::Client,
            request: reqwest::Request,
        ) -> Result<reqwest::Response> {
            assert_eq!(request.method(), reqwest::Method::GET);
            assert!(request.body().is_none());
            assert_eq!(request.headers().len(), 2);
            assert_eq!(
                request
                    .headers()
                    .get(header::USER_AGENT)
                    .and_then(|v| v.to_str().ok()),
                Some("Dakia/0.1 (remote email image proxy)")
            );
            assert_eq!(
                request.headers()[header::ACCEPT],
                "image/png,image/jpeg,image/gif,image/webp"
            );
            for name in ["cookie", "authorization", "referer", "proxy-authorization"] {
                assert!(!request.headers().contains_key(name));
            }
            self.sent
                .lock()
                .unwrap()
                .push(request.url().as_str().into());
            tokio::time::sleep(self.delay).await;
            let (headers, body) = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request");
            Ok(fixture(&headers, &body).await)
        }
    }
    fn redirect(location: &str) -> (String, Vec<u8>) {
        (
            format!("302 Found\r\nLocation: {location}\r\nContent-Length: 0"),
            vec![],
        )
    }
    fn png_reply() -> (String, Vec<u8>) {
        (
            "200 OK\r\nContent-Type: image/png".into(),
            b"\x89PNG\r\n\x1a\n".to_vec(),
        )
    }
    #[tokio::test]
    async fn relative_same_host_and_all_supported_redirect_statuses_work() {
        for status in [301, 302, 303, 307, 308] {
            let network = ScriptNetwork::new(vec![
                (
                    format!("{status} Redirect\r\nLocation: ../logo.png\r\nContent-Length: 0"),
                    vec![],
                ),
                redirect("//cdn.example.org/final.png"),
                png_reply(),
            ]);
            assert!(fetch_with_network(
                "https://images.example.org/path/start",
                &network,
                TOTAL_TIMEOUT
            )
            .await
            .is_ok());
            assert_eq!(
                *network.sent.lock().unwrap(),
                [
                    "https://images.example.org/path/start",
                    "https://images.example.org/logo.png",
                    "https://cdn.example.org/final.png"
                ]
            );
        }
    }
    #[tokio::test]
    async fn every_redirect_target_is_validated_before_dns_or_transport() {
        for location in [
            "http://cdn.example.org/logo",
            "https://user:pass@cdn.example.org/logo",
            "https://cdn.example.org:8443/logo",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://169.254.169.254/",
            "https://100.64.0.1/",
            "file:///etc/passwd",
            "https://[fd00::1]/",
            "https://[fe80::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://[",
            "",
            "https://cdn.example.org/a b",
        ] {
            let network = ScriptNetwork::new(vec![redirect(location)]);
            assert_eq!(
                fetch_with_network("https://images.example.org/start", &network, TOTAL_TIMEOUT)
                    .await
                    .unwrap_err()
                    .1,
                "remote_image_redirect_blocked",
                "{location}"
            );
            assert_eq!(network.hosts.lock().unwrap().len(), 1);
            assert_eq!(network.sent.lock().unwrap().len(), 1);
        }
        let network = ScriptNetwork::new(vec![redirect(&format!("/{}", "a".repeat(4096)))]);
        assert_eq!(
            fetch_with_network("https://images.example.org/start", &network, TOTAL_TIMEOUT)
                .await
                .unwrap_err()
                .1,
            "remote_image_redirect_blocked"
        );
    }
    #[tokio::test]
    async fn fresh_dns_rejects_private_mixed_rebinding_and_excessive_answers() {
        let good: SocketAddr = "8.8.8.8:443".parse().unwrap();
        let mut answers = vec![vec![], vec![good; 33]];
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.100.100.100",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:169.254.169.254",
        ] {
            let bad = SocketAddr::new(ip.parse().unwrap(), 443);
            answers.push(vec![bad]);
            answers.push(vec![good, bad]);
        }
        for addresses in answers {
            // Same host, new URL must resolve again, not reuse the first client.
            let network = ScriptNetwork::new(vec![redirect("/next")]);
            *network.dns.lock().unwrap() = vec![vec![good], addresses].into();
            assert_eq!(
                fetch_with_network("https://images.example.org/start", &network, TOTAL_TIMEOUT)
                    .await
                    .unwrap_err()
                    .1,
                "remote_image_redirect_blocked"
            );
            assert_eq!(network.hosts.lock().unwrap().len(), 2);
            assert_eq!(network.sent.lock().unwrap().len(), 1);
        }
    }
    #[tokio::test]
    async fn invalid_redirects_cycles_and_hop_limits_are_bounded() {
        for headers in [
            "302 Found\r\nContent-Length: 0",
            "300 Multiple Choices\r\nLocation: /next",
            "304 Not Modified\r\nLocation: /next",
            "305 Use Proxy\r\nLocation: /next",
        ] {
            let network = ScriptNetwork::new(vec![(headers.into(), vec![])]);
            assert_eq!(
                fetch_with_network("https://images.example.org/start", &network, TOTAL_TIMEOUT)
                    .await
                    .unwrap_err()
                    .1,
                "remote_image_redirect_blocked"
            );
        }
        for replies in [
            vec![redirect("/start#loop")],
            vec![redirect("/next"), redirect("/start")],
            (0..4).map(|i| redirect(&format!("/hop{i}"))).collect(),
        ] {
            let network = ScriptNetwork::new(replies);
            assert_eq!(
                fetch_with_network("https://images.example.org/start", &network, TOTAL_TIMEOUT)
                    .await
                    .unwrap_err()
                    .1,
                "remote_image_redirect_blocked"
            );
            assert!(network.sent.lock().unwrap().len() <= 4);
        }
        let network = ScriptNetwork::new(vec![
            redirect("/one"),
            redirect("/two"),
            redirect("/three"),
            png_reply(),
        ]);
        assert!(
            fetch_with_network("https://images.example.org/start", &network, TOTAL_TIMEOUT)
                .await
                .is_ok()
        );
    }
    #[tokio::test]
    async fn one_deadline_covers_the_entire_chain() {
        let mut network = ScriptNetwork::new(vec![redirect("/next"), png_reply()]);
        network.delay = Duration::from_millis(40);
        let start = tokio::time::Instant::now();
        assert_eq!(
            fetch_with_network(
                "https://images.example.org/start",
                &network,
                Duration::from_millis(65)
            )
            .await
            .unwrap_err()
            .1,
            "remote_image_unavailable"
        );
        assert!(start.elapsed() < Duration::from_millis(200));
        assert_eq!(network.sent.lock().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn final_content_rules_still_apply_after_redirects() {
        for (headers, body, expected) in [
            ("404 Not Found", b"".to_vec(), "remote_image_unavailable"),
            (
                "200 OK\r\nContent-Type: image/svg+xml",
                b"<svg/>".to_vec(),
                "remote_image_unsupported_content",
            ),
            (
                "200 OK\r\nContent-Type: image/png",
                b"<html>".to_vec(),
                "remote_image_unsupported_content",
            ),
            (
                "200 OK\r\nContent-Type: image/png\r\nContent-Length: 5242881",
                vec![],
                "remote_image_too_large",
            ),
            (
                "200 OK\r\nContent-Type: image/png",
                vec![b'a'; MAX_BYTES + 1],
                "remote_image_too_large",
            ),
        ] {
            let network = ScriptNetwork::new(vec![redirect("/next"), (headers.into(), body)]);
            assert_eq!(
                fetch_with_network("https://images.example.org/start", &network, TOTAL_TIMEOUT)
                    .await
                    .unwrap_err()
                    .1,
                expected
            );
        }
    }
    // Local fixture is used only for read_image. fetch still rejects it.
    async fn fixture(headers: &str, body: &[u8]) -> reqwest::Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut wire = format!("HTTP/1.1 {headers}\r\nConnection: close\r\n\r\n").into_bytes();
        wire.extend_from_slice(body);
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(
                socket.read(&mut request).await.unwrap() > 0,
                "fixture client disconnected before its request"
            );
            // Oversized responses may be abandoned before the fixture finishes.
            let _ = socket.write_all(&wire).await;
        });
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .get(format!("http://{address}/asset"))
            .send()
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn public_cdn_redirect_is_an_image_not_a_blanket_rejection() {
        let network = ScriptNetwork::new(vec![
            ("302 Found\r\nLocation: https://cdn.example.org/logo.png\r\nSet-Cookie: secret=value\r\nContent-Length: 0".into(), vec![]),
            ("200 OK\r\nContent-Type: image/x-png".into(), b"\x89PNG\r\n\x1a\n".to_vec()),
        ]);
        let image = fetch_with_network("https://images.example.org/logo", &network, TOTAL_TIMEOUT)
            .await
            .unwrap();
        assert!(image.starts_with("data:image/png;base64,"));
        assert_eq!(
            *network.hosts.lock().unwrap(),
            ["images.example.org", "cdn.example.org"]
        );
    }
    #[tokio::test]
    async fn representative_http_responses_are_canonical_and_fail_independently() {
        let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQAAAABJRU5ErkJggg==").unwrap();
        // Exact observed Swedbank logo MIME, followed by an unavailable sibling,
        // then a healthy independent image. No real sender/tracker is contacted.
        for mime in ["image/x-png;charset=utf-8", "image/png", "IMAGE/PNG"] {
            let headers = format!(
                "200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}",
                png.len()
            );
            let data = read_image(fixture(&headers, &png).await).await.unwrap();
            assert_eq!(
                data,
                format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(&png)
                )
            );
            let error = read_image(
                fixture(
                    "404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 7",
                    b"expired",
                )
                .await,
            )
            .await
            .unwrap_err();
            assert_eq!(
                (error.0, error.1),
                (StatusCode::BAD_GATEWAY, "remote_image_unavailable")
            );
        }
        for (headers, body, code) in [
            (
                "302 Found\r\nLocation: http://127.0.0.1/private\r\nContent-Length: 0",
                &b""[..],
                "remote_image_redirect_blocked",
            ),
            (
                "200 OK\r\nContent-Type: image/x-png\r\nContent-Length: 6",
                &b"<html>"[..],
                "remote_image_unsupported_content",
            ),
            (
                "200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: 6",
                &b"<svg/>"[..],
                "remote_image_unsupported_content",
            ),
            (
                "200 OK\r\nContent-Type: image/png\r\nContent-Length: 5242881",
                &b""[..],
                "remote_image_too_large",
            ),
            (
                "200 OK\r\nContent-Type: image/png\r\nContent-Length: 100",
                &b"short"[..],
                "remote_image_unavailable",
            ),
            (
                "200 OK\r\nContent-Length: 0",
                &b""[..],
                "remote_image_unsupported_content",
            ),
            (
                "204 No Content",
                &b""[..],
                "remote_image_unsupported_content",
            ),
        ] {
            assert_eq!(
                read_image(fixture(headers, body).await)
                    .await
                    .unwrap_err()
                    .1,
                code
            );
        }
        // Streaming without Content-Length is subject to the same byte limit.
        let body = vec![b'a'; MAX_BYTES + 1];
        let mut chunked = format!("{:x}\r\n", body.len()).into_bytes();
        chunked.extend(body);
        chunked.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(
            read_image(
                fixture(
                    "200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked",
                    &chunked
                )
                .await
            )
            .await
            .unwrap_err()
            .1,
            "remote_image_too_large"
        );
        assert_eq!(
            fetch("http://127.0.0.1/image").await.unwrap_err().1,
            "remote_image_rejected"
        );
    }
    #[test]
    fn legacy_raster_mime_aliases_require_matching_signatures() {
        for mime in [
            "image/png",
            "image/x-png;charset=utf-8",
            "IMAGE/PNG; charset=UTF-8",
        ] {
            assert_eq!(image_type(mime, b"\x89PNG\r\n\x1a\n"), Some("image/png"));
            assert_eq!(image_type(mime, b"<html>not an image</html>"), None);
        }
        for mime in ["image/jpeg", "image/jpg", "image/pjpeg"] {
            assert_eq!(image_type(mime, b"\xff\xd8\xff"), Some("image/jpeg"));
            assert_eq!(image_type(mime, b"\x89PNG\r\n\x1a\n"), None);
        }
        for mime in [
            "text/html",
            "application/octet-stream",
            "image/svg+xml",
            "image/x-png-evil",
        ] {
            assert_eq!(image_type(mime, b"\x89PNG\r\n\x1a\n"), None);
        }
    }
    #[test]
    fn dangerous_targets_and_active_formats_are_rejected() {
        for value in [
            "http://example.com/a.png",
            "file:///etc/passwd",
            "https://owner:secret@example.com/a.png",
            "https://example.com:8443/a.png",
            "https://127.0.0.1/a.png",
            "https://[::1]/a.png",
        ] {
            assert!(target(value).is_err(), "{value}");
        }
        assert!(target("https://example.com/a.png?q=1").is_ok());
        for value in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::ffff:8.8.8.8",
            "fd00::1",
            "fe80::1",
            "2001:db8::1",
            "2002::1",
        ] {
            assert!(!public(value.parse().unwrap()), "{value}");
        }
        for value in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public(value.parse().unwrap()), "{value}");
        }
        assert_eq!(
            image_type("image/png", b"\x89PNG\r\n\x1a\n"),
            Some("image/png")
        );
        assert_eq!(
            image_type("image/svg+xml", b"<svg onload='alert(1)'/>"),
            None
        );
        assert_eq!(image_type("image/png", b"<html>not a png</html>"), None);
    }
    #[tokio::test]
    #[ignore = "synthetic loopback coordinator fixture for playwright.images.config.ts"]
    async fn browser_image_redirect_fixture() {
        async fn image(Json(input): Json<RemoteImageRequest>) -> Result<Json<String>> {
            let initial = target(&input.url)?;
            if initial.host_str() != Some("images.example.org") {
                return Err(blocked());
            }
            let replies = if initial.path() == "/failure.png" {
                vec![
                    redirect("https://cdn.example.org/expired.png"),
                    ("404 Not Found".into(), vec![]),
                ]
            } else {
                let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAFAAAAAgCAIAAAAKUWg7AAAAQklEQVR4nO3PAQkAIBDAQCMawKxmM4awP1iA3Tr7jmp9PwAGBgYGBgYeE3A94HrA9YDrAdcDrgdcD7gecD3gesD1HgWJJNNntUVKAAAAAElFTkSuQmCC").unwrap();
                vec![
                    redirect("/relative.png"),
                    redirect("https://cdn.example.org/final.png"),
                    ("200 OK\r\nContent-Type: image/x-png".into(), png),
                ]
            };
            Ok(Json(
                fetch_with_network(&input.url, &ScriptNetwork::new(replies), TOTAL_TIMEOUT).await?,
            ))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:4339")
            .await
            .unwrap();
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/healthz",
                    axum::routing::get(|| async { "synthetic images ready" }),
                )
                .route("/image", axum::routing::post(image)),
        )
        .await
        .unwrap();
    }
    #[tokio::test]
    #[ignore = "explicit public-network synthetic image check"]
    async fn actual_https_png_is_bounded_and_inert() {
        let image = fetch("https://httpbin.org/image/png").await.unwrap();
        assert!(image.starts_with("data:image/png;base64,iVBORw0KGgo"));
    }
    #[tokio::test]
    #[ignore = "explicit nonpersonal static raster diagnostic; never use tracking URLs"]
    async fn public_raster_production_diagnostic() {
        struct DiagnosticNetwork;
        #[async_trait::async_trait]
        impl ImageNetwork for DiagnosticNetwork {
            async fn resolve(&self, host: &str) -> Result<Vec<SocketAddr>> {
                let addresses = PublicNetwork.resolve(host).await?;
                println!(
                    "image DNS: answers={} all_public={}",
                    addresses.len(),
                    addresses.iter().all(|a| public(a.ip()))
                );
                Ok(addresses)
            }
            async fn send(
                &self,
                client: reqwest::Client,
                request: reqwest::Request,
            ) -> Result<reqwest::Response> {
                match client.execute(request).await {
                    Ok(response) => {
                        println!("image HTTP: status={}", response.status().as_u16());
                        Ok(response)
                    }
                    Err(error) => {
                        println!(
                            "image transport: timeout={} connect={} detail={}",
                            error.is_timeout(),
                            error.is_connect(),
                            error.without_url()
                        );
                        Err(unavailable())
                    }
                }
            }
        }
        let value =
            std::env::var("DAKIA_TEST_PUBLIC_IMAGE_URL").expect("nonpersonal static URL required");
        let image = fetch_with_network(&value, &DiagnosticNetwork, TOTAL_TIMEOUT)
            .await
            .unwrap();
        assert!([
            "data:image/png;base64,",
            "data:image/jpeg;base64,",
            "data:image/gif;base64,",
            "data:image/webp;base64,"
        ]
        .iter()
        .any(|p| image.starts_with(p)));
        println!(
            "image validated: kind={} encoded_length={}",
            image.split(';').next().unwrap(),
            image.len()
        );
    }
    #[tokio::test]
    #[ignore = "explicit read-only check of a nonpersonal public PNG URL"]
    async fn approved_public_png_uses_the_production_fetcher() {
        let value =
            std::env::var("DAKIA_TEST_PUBLIC_IMAGE_URL").expect("public fixture URL required");
        let image = fetch(&value).await.unwrap();
        assert!(image.starts_with("data:image/png;base64,iVBORw0KGgo"));
    }
}
