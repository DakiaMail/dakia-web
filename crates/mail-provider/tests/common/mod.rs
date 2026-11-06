#![allow(dead_code)]
use dakia_mail_provider::*;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use tokio_native_tls::{TlsAcceptor, TlsStream};
pub type Wire = BufReader<TlsStream<TcpStream>>;
pub struct Fixture {
    pub listener: TcpListener,
    pub acceptor: TlsAcceptor,
    pub cert: Vec<u8>,
    pub pem: String,
}
impl Fixture {
    pub async fn new() -> Self {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pem = cert.pem();
        // Security.framework cannot import rcgen's EC PKCS#8 identity directly.
        // Keep the same ephemeral certificate and use its supported PKCS#12 path.
        #[cfg(target_os = "macos")]
        let identity = {
            let key =
                openssl::pkey::PKey::private_key_from_pem(signing_key.serialize_pem().as_bytes())
                    .unwrap();
            let certificate = openssl::x509::X509::from_pem(pem.as_bytes()).unwrap();
            let archive = openssl::pkcs12::Pkcs12::builder()
                .name("dakia-disposable-fixture")
                .pkey(&key)
                .cert(&certificate)
                .build2("fixture-only")
                .unwrap()
                .to_der()
                .unwrap();
            native_tls::Identity::from_pkcs12(&archive, "fixture-only").unwrap()
        };
        #[cfg(not(target_os = "macos"))]
        let identity = native_tls::Identity::from_pkcs8(
            pem.as_bytes(),
            signing_key.serialize_pem().as_bytes(),
        )
        .unwrap();
        Self {
            listener: TcpListener::bind("127.0.0.1:0").await.unwrap(),
            acceptor: TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap()),
            cert: cert.der().to_vec(),
            pem,
        }
    }
    pub fn config(&self, security: Security) -> Config {
        let endpoint = Endpoint {
            host: "localhost".into(),
            port: self.listener.local_addr().unwrap().port(),
            security,
        };
        Config {
            account_id: "fixture-account".into(),
            primary_address: "sender@example.test".into(),
            kind: ProviderKind::Imap,
            imap: endpoint.clone(),
            smtp: endpoint,
            timeout: Duration::from_secs(3),
            full_imap_exposure_confirmed: false,
            folders: FolderMapping {
                sent: Some("Sent".into()),
                archive: Some("Archive".into()),
                trash: Some("Trash".into()),
            },
        }
    }
    pub async fn accept(self, starttls: bool, smtp: bool) -> Wire {
        let (tcp, _) = self.listener.accept().await.unwrap();
        let tcp = if starttls {
            let mut r = BufReader::new(tcp);
            r.get_mut()
                .write_all(if smtp {
                    b"220 fixture ESMTP\r\n"
                } else {
                    b"* OK fixture\r\n"
                })
                .await
                .unwrap();
            let mut line = String::new();
            r.read_line(&mut line).await.unwrap();
            if smtp {
                assert!(line.starts_with("EHLO "));
                r.get_mut()
                    .write_all(b"250-localhost\r\n250 STARTTLS\r\n")
                    .await
                    .unwrap();
                line.clear();
                r.read_line(&mut line).await.unwrap();
                assert_eq!(line, "STARTTLS\r\n");
                r.get_mut().write_all(b"220 ready\r\n").await.unwrap();
            } else {
                assert_eq!(line, "DSTART STARTTLS\r\n");
                r.get_mut().write_all(b"DSTART OK ready\r\n").await.unwrap();
            }
            r.into_inner()
        } else {
            tcp
        };
        BufReader::new(self.acceptor.accept(tcp).await.unwrap())
    }
}
pub fn credentials() -> Credentials {
    Credentials {
        username: "fixture".into(),
        password: "fixture-only-password".into(),
    }
}
pub async fn line(w: &mut Wire) -> String {
    let mut line = String::new();
    w.read_line(&mut line).await.unwrap();
    line
}
pub async fn reply(w: &mut Wire, s: &str) {
    w.get_mut().write_all(s.as_bytes()).await.unwrap();
    w.get_mut().flush().await.unwrap();
}
pub async fn command(w: &mut Wire, expected: &str, response: &str) {
    let line = line(w).await;
    let (tag, cmd) = line.trim_end().split_once(' ').unwrap();
    assert_eq!(cmd, expected);
    reply(w, &response.replace("$TAG", tag)).await;
}
pub async fn imap_login(w: &mut Wire, greeted: bool) {
    if !greeted {
        reply(w, "* OK fixture\r\n").await;
    }
    command(
        w,
        "LOGIN \"fixture\" \"fixture-only-password\"",
        "$TAG OK authenticated\r\n",
    )
    .await;
}
pub async fn smtp_login(w: &mut Wire, greeted: bool) {
    if !greeted {
        reply(w, "220 fixture ESMTP\r\n").await;
    }
    assert!(line(w).await.starts_with("EHLO "));
    reply(w, "250-localhost\r\n250-AUTH PLAIN\r\n250 8BITMIME\r\n").await;
    assert!(line(w).await.starts_with("AUTH PLAIN "));
    reply(w, "235 authenticated\r\n").await;
}
pub fn compose() -> dakia_mail_provider::mime::Compose {
    dakia_mail_provider::mime::Compose {
        to: vec!["to@example.test".into()],
        cc: vec![],
        bcc: vec!["hidden@example.test".into()],
        subject: "Dakia fixture".into(),
        text: "plain fixture".into(),
        html: Some("<p>HTML fixture</p>".into()),
        message_id: "<immutable-fixture@example.test>".into(),
        in_reply_to: Some("<parent@example.test>".into()),
        references: Some("<ancestor@example.test> <parent@example.test>".into()),
        attachments: vec![dakia_mail_provider::mime::ComposeAttachment {
            filename: "test.txt".into(),
            content_type: "text/plain".into(),
            bytes: b"attachment bytes\nLF\r\nCRLF".to_vec(),
        }],
    }
}
