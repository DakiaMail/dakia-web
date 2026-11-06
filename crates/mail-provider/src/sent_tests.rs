use super::*;
fn folder(name: &str, role: bool) -> Folder {
    Folder {
        name: name.into(),
        selectable: true,
        attributes: if role { vec!["Sent".into()] } else { vec![] },
    }
}
#[test]
fn roles_names_evidence_and_explicit_mapping() {
    let names = vec![
        folder("Sent", true),
        folder("Sent Messages", true),
        folder("Sent Items", false),
    ];
    assert_eq!(
        sent_candidates(&names, None).unwrap(),
        vec!["Sent", "Sent Messages"]
    );
    let counts = vec![("Sent".into(), 0), ("Sent Messages".into(), 7)];
    assert_eq!(choose_sent(&counts, None).unwrap(), "Sent Messages");
    assert_eq!(choose_sent(&counts, Some("Sent")).unwrap(), "Sent");
    for counts in [
        vec![("Sent Messages".into(), 0), ("Sent".into(), 0)],
        vec![("Sent".into(), 2), ("Sent Messages".into(), 2)],
    ] {
        assert_eq!(choose_sent(&counts, None).unwrap(), "Sent");
    }
    assert!(sent_candidates(&names, Some("missing")).is_err());
    let mut disabled = folder("Sent", true);
    disabled.selectable = false;
    assert!(sent_candidates(&[disabled], None).is_err());
    assert!(sent_candidates(&[folder("INBOX", false)], None).is_err());
    assert_eq!(
        sent_candidates(&[folder("INBOX.Sent Items", false)], None).unwrap(),
        vec!["INBOX.Sent Items"]
    );
    assert_eq!(
        sent_candidates(&[folder("Envoyes", true)], None).unwrap(),
        vec!["Envoyes"]
    );
    assert!(choose_sent(&[], None).is_err());
}
