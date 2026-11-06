use super::*;
use mail_auth::{
    common::{crypto::Ed25519Key, parse::TxtRecordParser, verify::DomainKey},
    dkim::DkimSigner,
    Parameters, ResolverCache, Txt,
};
use std::{borrow::Borrow, collections::HashMap, hash::Hash, time::Instant};
struct LocalDns(HashMap<Box<str>, Txt>);
impl ResolverCache<Box<str>, Txt> for LocalDns {
    fn get<Q>(&self, key: &Q) -> Option<Txt>
    where
        Box<str>: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.0.get(key).cloned()
    }
    fn remove<Q>(&self, _: &Q) -> Option<Txt>
    where
        Box<str>: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        None
    }
    fn insert(&self, _: Box<str>, _: Txt, _: Instant) {}
}
#[tokio::test]
async fn actual_dkim_signature_binds_both_headers_and_detects_tampering() {
    let key_bytes = Ed25519Key::generate_pkcs8().unwrap();
    let key = Ed25519Key::from_pkcs8_der(&key_bytes).unwrap();
    let record = format!(
        "v=DKIM1; k=ed25519; p={}",
        base64::engine::general_purpose::STANDARD.encode(key.public_key())
    );
    let dns = LocalDns(HashMap::from([(
        "fixture._domainkey.example.org.".into(),
        Txt::DomainKey(std::sync::Arc::new(
            DomainKey::parse(record.as_bytes()).unwrap(),
        )),
    )]));
    let auth = MessageAuthenticator::new_system_conf().unwrap();
    let raw = b"From: List <list@example.org>\r\nTo: Reader <reader@example.net>\r\nSubject: Synthetic newsletter\r\nList-Unsubscribe: <https://example.org/leave?fixture=1>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\nSynthetic content only.\r\n";
    for both in [false, true] {
        let key = Ed25519Key::from_pkcs8_der(&key_bytes).unwrap();
        let headers = if both {
            vec![
                "From",
                "Subject",
                "List-Unsubscribe",
                "List-Unsubscribe-Post",
            ]
        } else {
            vec!["From", "Subject", "List-Unsubscribe"]
        };
        let signature = DkimSigner::from_key(key)
            .domain("example.org")
            .selector("fixture")
            .headers(headers)
            .sign(raw)
            .unwrap();
        let mut signed = Vec::new();
        signature.write(&mut signed, true);
        signed.extend_from_slice(raw);
        let message = AuthenticatedMessage::parse(&signed).unwrap();
        let results = auth
            .verify_dkim(Parameters::new(&message).with_txt_cache(&dns))
            .await;
        assert!(results
            .iter()
            .any(|r| matches!(r.result(), DkimResult::Pass)));
        assert_eq!(covered(&results), both);
        assert_eq!(
            matches!(
                select(&signed, covered(&results)),
                Some(Action::OneClick { .. })
            ),
            both
        );
        let changed = String::from_utf8(signed)
            .unwrap()
            .replace("fixture=1", "fixture=2");
        let changed = AuthenticatedMessage::parse(changed.as_bytes()).unwrap();
        let results = auth
            .verify_dkim(Parameters::new(&changed).with_txt_cache(&dns))
            .await;
        assert!(!covered(&results));
    }
}
