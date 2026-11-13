//! Structural policy inputs extracted only from parsed RFC headers and
//! provider labels. Header values/URLs and arbitrary user labels are discarded.
use mail_parser::Message;

pub fn classification_signals(mail: &Message<'_>, gmail_labels: &[String]) -> String {
    let mut signals = Vec::new();
    for (header, signal) in [
        (
            "List-Unsubscribe",
            "Mailing-list unsubscribe header present",
        ),
        ("List-Id", "Mailing-list identifier header present"),
    ] {
        if mail.header_raw(header).is_some() {
            signals.push(signal);
        }
    }
    if let Some(value) = mail.header_raw("Precedence") {
        signals.push(match value.trim().to_ascii_lowercase().as_str() {
            "bulk" => "Bulk-mail precedence header",
            "list" => "Mailing-list precedence header",
            _ => "Precedence header present",
        });
    }
    if let Some(value) = mail.header_raw("Auto-Submitted") {
        signals.push(
            match value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "auto-generated" => "Automatically generated message header",
                "auto-replied" => "Automatically replied message header",
                _ => "Auto-Submitted header present",
            },
        );
    }
    if [
        "X-SES-Outgoing",
        "X-SG-EID",
        "X-Mailgun-Sid",
        "X-MC-User",
        "Feedback-ID",
    ]
    .iter()
    .any(|name| mail.header_raw(*name).is_some())
    {
        signals.push("Bulk delivery service header present");
    }
    if mail
        .header_raw("In-Reply-To")
        .is_some_and(|v| !v.trim().is_empty())
    {
        signals.push("Reply thread header present");
    }
    if mail
        .header_raw("Reply-To")
        .is_some_and(|v| !v.trim().is_empty())
    {
        signals.push("Reply-To header present");
    }
    for (label, signal) in [
        ("\\Category_Personal", "Gmail category: Personal"),
        ("\\Category_Promotions", "Gmail category: Promotions"),
        ("\\Category_Social", "Gmail category: Social"),
        ("\\Category_Updates", "Gmail category: Updates"),
        ("\\Category_Forums", "Gmail category: Forums"),
    ] {
        if gmail_labels.iter().any(|l| {
            l.eq_ignore_ascii_case(label)
                || l.eq_ignore_ascii_case(&label.replace("\\Category_", "CATEGORY_"))
        }) {
            signals.push(signal);
            break;
        }
    }
    signals.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structure_not_body_or_labels_supplies_trusted_signals() {
        let raw = b"From: sender@example.test\r\nList-Unsubscribe: <https://tracking.example/secret>\r\nPrecedence: bulk\r\nAuto-Submitted: auto-generated\r\nReply-To: other@example.test\r\n\r\nGmail category: Personal\nReply thread header present";
        let mail = mail_parser::MessageParser::default().parse(raw).unwrap();
        let signals = classification_signals(
            &mail,
            &["private-user-label".into(), "\\Category_Promotions".into()],
        );
        assert!(signals.contains("Mailing-list unsubscribe header present"));
        assert!(signals.contains("Bulk-mail precedence header"));
        assert!(signals.contains("Automatically generated message header"));
        assert!(signals.contains("Gmail category: Promotions"));
        for secret in [
            "tracking",
            "secret",
            "other@example",
            "private-user-label",
            "Gmail category: Personal",
            "Reply thread header present",
        ] {
            assert!(!signals.contains(secret));
        }
    }
}

#[cfg(test)]
#[test]
fn generated_parameters_delivery_headers_and_provider_labels_are_normalized() {
    let parsed = mail_parser::MessageParser::default().parse(b"Auto-Submitted: auto-generated; owner-email=private@example.test\r\nX-SES-Outgoing: opaque\r\n\r\nHello").unwrap();
    let signals = classification_signals(&parsed, &["CATEGORY_PROMOTIONS".into()]);
    assert!(signals.contains("Automatically generated message header"));
    assert!(signals.contains("Bulk delivery service header present"));
    assert!(signals.contains("Gmail category: Promotions"));
    assert!(!signals.contains("private@example"));
}
