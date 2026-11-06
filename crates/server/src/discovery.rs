//! Read-only, credential-free setup discovery. Only a domain leaves this server.
//! Fixed HTTPS ISPDB origin, no redirects/proxies, no guessed server names/ports.
use crate::*;
use hickory_resolver::{config::ResolverOpts, TokioAsyncResolver};
use std::time::Duration;
use tokio::sync::Semaphore;
static LANES: Semaphore = Semaphore::const_new(4);
const MAX_XML: usize = 128 * 1024;

fn domain(value: &str) -> Option<String> {
    if value.len() > 253 || !value.contains('.') || value.ends_with('.') {
        return None;
    }
    let value = value.to_ascii_lowercase();
    if value.split('.').any(|part| {
        part.is_empty()
            || part.len() > 63
            || part.starts_with('-')
            || part.ends_with('-')
            || !part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    }) || value.parse::<std::net::IpAddr>().is_ok()
    {
        return None;
    }
    Some(value)
}
fn email_domain(email: &str) -> Option<String> {
    let (local, host) = email.split_once('@')?;
    if email.len() > 320
        || local.is_empty()
        || local.contains('@')
        || email.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    domain(host)
}
fn known(host: &str, email: &str) -> Option<DiscoveredSettings> {
    let (provider, imap, smtp) = match host {
        "gmail.com" | "googlemail.com" => (ProviderKind::Gmail, "imap.gmail.com", "smtp.gmail.com"),
        "migadu.com" => (ProviderKind::Imap, "imap.migadu.com", "smtp.migadu.com"),
        _ => return None,
    };
    Some(DiscoveredSettings {
        provider,
        imap_host: imap.into(),
        imap_port: 993,
        imap_tls: TlsMode::Implicit,
        smtp_host: smtp.into(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        username: email.into(),
        source: "provider".into(),
    })
}
fn mx_provider(host: &str) -> Option<&'static str> {
    match host.trim_end_matches('.').to_ascii_lowercase().as_str() {
        "aspmx1.migadu.com" | "aspmx2.migadu.com" => Some("migadu.com"),
        "aspmx.l.google.com"
        | "alt1.aspmx.l.google.com"
        | "alt2.aspmx.l.google.com"
        | "alt3.aspmx.l.google.com"
        | "alt4.aspmx.l.google.com"
        | "smtp.google.com" => Some("gmail.com"),
        _ => None,
    }
}
fn parse(xml: &str, email: &str, host: &str) -> Option<DiscoveredSettings> {
    // roxmltree rejects DTDs by default and never retrieves external entities.
    let doc = roxmltree::Document::parse(xml).ok()?;
    let provider = doc.descendants().find(|n| {
        n.has_tag_name("emailProvider")
            && n.children().any(|c| {
                c.has_tag_name("domain") && c.text().is_some_and(|v| v.eq_ignore_ascii_case(host))
            })
    })?;
    let expand = |value: &str| -> Option<String> {
        let local = email.split_once('@')?.0;
        let value = value
            .replace("%EMAILADDRESS%", email)
            .replace("%EMAILLOCALPART%", local)
            .replace("%EMAILDOMAIN%", host);
        (!value.contains('%')
            && !value.is_empty()
            && value.len() <= 320
            && !value.chars().any(char::is_control))
        .then_some(value)
    };
    let endpoint = |tag: &str, kind: &str| {
        provider
            .children()
            .filter(|n| n.has_tag_name(tag) && n.attribute("type") == Some(kind))
            .find_map(|node| {
                let field = |name: &str| {
                    node.children()
                        .find(|n| n.has_tag_name(name))
                        .and_then(|n| n.text())
                };
                if !node.children().any(|n| {
                    n.has_tag_name("authentication") && n.text() == Some("password-cleartext")
                }) {
                    return None;
                }
                let host = domain(&expand(field("hostname")?)?)?;
                let port: u16 = field("port")?.parse().ok()?;
                let tls = match (kind, field("socketType")?, port) {
                    ("imap", "SSL", 993) | ("smtp", "SSL", 465) => TlsMode::Implicit,
                    ("imap", "STARTTLS", 143) | ("smtp", "STARTTLS", 587) => TlsMode::Starttls,
                    _ => return None,
                };
                Some((host, port, tls, expand(field("username")?)?))
            })
    };
    let (imap_host, imap_port, imap_tls, username) = endpoint("incomingServer", "imap")?;
    let (smtp_host, smtp_port, smtp_tls, smtp_user) = endpoint("outgoingServer", "smtp")?;
    // The current account contract has one username, not separate SMTP credentials.
    if username != smtp_user {
        return None;
    }
    Some(DiscoveredSettings {
        provider: ProviderKind::Imap,
        imap_host,
        imap_port,
        imap_tls,
        smtp_host,
        smtp_port,
        smtp_tls,
        username,
        source: "thunderbird".into(),
    })
}
async fn ispdb(email: &str, host: &str) -> Option<DiscoveredSettings> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;
    // host was validated as DNS labels, so it cannot inject a URL/path/query.
    let mut response = client
        .get(format!("https://autoconfig.thunderbird.net/v1.1/{host}"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|n| n > MAX_XML as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > MAX_XML {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(std::str::from_utf8(&bytes).ok()?, email, host)
}
async fn lookup(email: &str, host: &str) -> Option<DiscoveredSettings> {
    if let Some(settings) = known(host, email) {
        return Some(settings);
    }
    let mx = async {
        let (config, mut options) = hickory_resolver::system_conf::read_system_conf().ok()?;
        options.timeout = Duration::from_secs(2);
        options.attempts = 1;
        options.cache_size = ResolverOpts::default().cache_size;
        let resolver = TokioAsyncResolver::tokio(config, options);
        let records = resolver.mx_lookup(format!("{host}.")).await.ok()?;
        // Conflicting/unknown MX providers are uncertain: retain manual fallback.
        let providers: Option<Vec<_>> = records
            .iter()
            .map(|r| mx_provider(&r.exchange().to_utf8()))
            .collect();
        let providers = providers?;
        let first = *providers.first()?;
        if providers.iter().any(|p| *p != first) {
            return None;
        }
        known(first, email).map(|mut settings| {
            settings.source = "mx".into();
            settings
        })
    };
    let (direct, hosted) = tokio::join!(ispdb(email, host), mx);
    direct.or(hosted)
}
pub(crate) async fn discover(
    Json(input): Json<DiscoverAccount>,
) -> Result<Json<Option<DiscoveredSettings>>> {
    let host =
        email_domain(&input.email).ok_or(ApiFailure(StatusCode::BAD_REQUEST, "invalid_email"))?;
    let _lane = LANES
        .try_acquire()
        .map_err(|_| ApiFailure(StatusCode::TOO_MANY_REQUESTS, "discovery_busy"))?;
    let settings = tokio::time::timeout(Duration::from_secs(6), lookup(&input.email, &host))
        .await
        .ok()
        .flatten();
    Ok(Json(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    const XML: &str = r#"<clientConfig version="1.1"><emailProvider id="example.org"><domain>example.org</domain><incomingServer type="imap"><hostname>imap.example.org</hostname><port>993</port><socketType>SSL</socketType><authentication>password-cleartext</authentication><username>%EMAILADDRESS%</username></incomingServer><outgoingServer type="smtp"><hostname>smtp.example.org</hostname><port>587</port><socketType>STARTTLS</socketType><authentication>password-cleartext</authentication><username>%EMAILADDRESS%</username></outgoingServer></emailProvider></clientConfig>"#;
    #[test]
    fn secure_config_expansion_and_rejection() {
        let settings = parse(XML, "person@example.org", "example.org").unwrap();
        assert_eq!(settings.username, "person@example.org");
        assert_eq!(settings.smtp_tls, TlsMode::Starttls);
        for bad in [
            XML.replace("SSL", "plain"),
            XML.replace("993", "0"),
            XML.replace("imap.example.org", "127.0.0.1"),
            XML.replace("password-cleartext", "OAuth2"),
            XML.replace("%EMAILADDRESS%", "%UNKNOWN%"),
            XML.replace("<domain>example.org</domain>", "<domain>other.org</domain>"),
        ] {
            assert!(parse(&bad, "person@example.org", "example.org").is_none());
        }
        assert!(parse(
            "<!DOCTYPE foo [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><foo>&x;</foo>",
            "p@example.org",
            "example.org"
        )
        .is_none());
    }
    #[test]
    fn domain_and_mx_are_exact_not_suffix_guesses() {
        for email in [
            "x@localhost",
            "x@127.0.0.1",
            "x@example.com/path",
            "x@example.com?evil",
            "x@a..com",
            "x@-a.com",
            "x@y@a.com",
            "x\n@a.com",
        ] {
            assert!(email_domain(email).is_none());
        }
        assert_eq!(email_domain("x@EXAMPLE.ORG"), Some("example.org".into()));
        assert_eq!(mx_provider("aspmx1.migadu.com."), Some("migadu.com"));
        assert_eq!(mx_provider("aspmx1.migadu.com.evil.org"), None);
        assert_eq!(known("gmail.com", "p@gmail.com").unwrap().imap_port, 993);
        assert_eq!(
            known("migadu.com", "p@custom.org").unwrap().username,
            "p@custom.org"
        );
    }
    #[tokio::test]
    #[ignore = "explicit credential-free public ISPDB network acceptance"]
    async fn actual_ispdb_settings() {
        let settings = ispdb("synthetic@yahoo.com", "yahoo.com").await.unwrap();
        assert_eq!(settings.imap_host, "imap.mail.yahoo.com");
        assert_eq!(settings.imap_tls, TlsMode::Implicit);
    }
}
