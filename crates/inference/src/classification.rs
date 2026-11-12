//! Bounded deterministic mail detection. No models, network calls, or account state.
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use dakia_domain::*;
use regex::Regex;
use scraper::{Html, Selector};
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub const POLICY_REVISION: &str = "personal-first-1";
const MAX_TEXT: usize = 16_000;
#[derive(Debug, Clone)]
pub struct EmailClassificationInput {
    pub from_name: Option<String>,
    pub from_address: String,
    pub subject: String,
    pub body_text: String,
    pub classification_signals: String,
    pub known_correspondence: bool,
    pub received_at: String,
    pub html: String,
    pub calendar: String,
}
impl EmailClassificationInput {
    pub fn new(
        name: Option<&str>,
        address: &str,
        subject: &str,
        body: &str,
        signals: &str,
    ) -> Self {
        Self {
            from_name: name.map(str::to_owned),
            from_address: address.into(),
            subject: subject.chars().take(1000).collect(),
            body_text: body.chars().take(MAX_TEXT).collect(),
            classification_signals: signals.into(),
            known_correspondence: false,
            received_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            html: String::new(),
            calendar: String::new(),
        }
    }
    pub fn with_known_correspondence(mut self, known: bool) -> Self {
        self.known_correspondence = known;
        self
    }
}
#[derive(Default)]
pub struct LocalEmailClassifier;
impl LocalEmailClassifier {
    pub fn revision(&self) -> &str {
        POLICY_REVISION
    }
    pub fn classify(
        &mut self,
        inputs: &[EmailClassificationInput],
    ) -> anyhow::Result<Vec<Classification>> {
        Ok(inputs.iter().map(detect).collect())
    }
}
fn has(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| text.contains(p))
}
fn current_text(text: &str) -> String {
    text.lines()
        .take_while(|line| {
            let l = line.trim().to_lowercase();
            !(l.starts_with("on ") && l.ends_with("wrote:"))
                && !l.starts_with("-----original message")
        })
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TEXT)
        .collect()
}
fn visible_html(doc: &Html) -> String {
    doc.tree
        .root()
        .descendants()
        .filter_map(|node| {
            let scraper::Node::Text(text) = node.value() else {
                return None;
            };
            let quoted = node.ancestors().any(|a| {
                a.value().as_element().is_some_and(|e| {
                    matches!(
                        e.name(),
                        "script" | "style" | "blockquote" | "head" | "template"
                    ) || e.attr("hidden").is_some()
                        || e.attr("class")
                            .is_some_and(|c| has(c, &["gmail_quote", "yahoo_quoted"]))
                        || e.attr("id").is_some_and(|id| id == "divRplyFwdMsg")
                })
            });
            (!quoted).then_some(text.as_ref())
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TEXT)
        .collect()
}
fn regex(pattern: &str, text: &str) -> Option<String> {
    Regex::new(pattern)
        .expect("reviewed detector regex")
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().into())
}
fn detail(
    out: &mut Classification,
    kind: DetectedType,
    label: &str,
    value: String,
    action: Option<DetailAction>,
) {
    if !value.is_empty()
        && value.len() <= 2048
        && !value.chars().any(char::is_control)
        && out.details.len() < 16
    {
        out.details.push(MailDetail {
            email_type: kind,
            label: label.into(),
            value,
            action,
        });
    }
}
fn attention(out: &mut Classification, label: &str, expires_at: Option<String>) {
    if !out.attention.iter().any(|a| a.label == label) {
        out.attention.push(AttentionReason {
            label: label.into(),
            expires_at,
        });
    }
}
fn add(out: &mut Classification, kind: DetectedType) {
    if !out.types.contains(&kind) {
        out.types.push(kind);
    }
}
pub fn safe_link(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_lowercase();
    matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none()
        && !host.is_empty()
        && host != "localhost"
        && !host.ends_with(".localhost")
        && !host.ends_with(".local")
        && host.parse::<std::net::IpAddr>().is_err()
        && !host.starts_with('[')
        && !value.chars().any(char::is_control)
}
fn structured(value: &Value, out: &mut Classification, depth: usize) {
    if depth > 8 {
        return;
    }
    if let Some(items) = value.as_array() {
        for v in items.iter().take(30) {
            structured(v, out, depth + 1);
        }
        return;
    }
    let Some(obj) = value.as_object() else { return };
    let kind = obj
        .get("@type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("");
    let t = match kind {
        "Order" => Some(DetectedType::Order),
        "Invoice" => Some(DetectedType::Invoice),
        "ParcelDelivery" => Some(DetectedType::Shipping),
        "FlightReservation" | "Flight" => Some(DetectedType::FlightBooking),
        "EventReservation" | "Event" => Some(DetectedType::EventInvitation),
        _ => None,
    };
    if let Some(t) = t {
        add(out, t.clone());
        for (key, label, copy) in [
            ("orderNumber", "Order", true),
            ("confirmationNumber", "Reference", true),
            ("reservationNumber", "Reference", true),
            ("trackingNumber", "Tracking", true),
            ("flightNumber", "Flight", false),
            ("startDate", "Starts", false),
            ("departureTime", "Departs", false),
        ] {
            if let Some(value) = obj.get(key).and_then(Value::as_str) {
                detail(
                    out,
                    t.clone(),
                    label,
                    value.into(),
                    copy.then_some(DetailAction::Copy),
                );
            }
        }
        if kind == "ParcelDelivery" {
            if let Some(link) = obj
                .get("trackingUrl")
                .and_then(Value::as_str)
                .filter(|s| safe_link(s))
            {
                detail(
                    out,
                    t.clone(),
                    "Track shipment",
                    link.into(),
                    Some(DetailAction::Open),
                );
            }
        }
        if obj
            .get("reservationStatus")
            .and_then(Value::as_str)
            .is_some_and(|s| s.ends_with("ReservationCancelled"))
        {
            attention(out, "Booking cancelled", None);
        }
        if let Some(v) = obj.get("reservationFor") {
            structured(v, out, depth + 1);
        }
    }
    if let Some(v) = obj.get("@graph") {
        structured(v, out, depth + 1);
    }
}
fn calendar(input: &str, out: &mut Classification) {
    let unfolded = input.replace("\r\n ", "").replace("\r\n\t", "");
    if !unfolded.lines().any(|l| l.trim() == "BEGIN:VEVENT") {
        return;
    }
    add(out, DetectedType::EventInvitation);
    for line in unfolded.lines().take(200) {
        if let Some(value) = line.strip_prefix("SUMMARY:") {
            detail(
                out,
                DetectedType::EventInvitation,
                "Event",
                value.trim().into(),
                None,
            );
        }
        if let Some(value) = line.strip_prefix("DTSTART:") {
            if let Ok(time) = chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y%m%dT%H%M%SZ")
            {
                detail(
                    out,
                    DetectedType::EventInvitation,
                    "Starts",
                    time.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true),
                    None,
                );
            }
        }
        if line.trim() == "METHOD:CANCEL" || line.trim() == "STATUS:CANCELLED" {
            attention(out, "Event cancelled", None);
        }
    }
}
pub fn detect(input: &EmailClassificationInput) -> Classification {
    let mut out = Classification {
        status: CategoryStatus::Ready,
        policy_revision: POLICY_REVISION.into(),
        ..Default::default()
    };
    let html: String = input.html.chars().take(256_000).collect();
    let doc = Html::parse_document(&html);
    let body = if input.body_text.trim().is_empty() {
        visible_html(&doc)
    } else {
        current_text(&input.body_text)
    };
    let own = format!("{} {}", input.subject, body);
    let text = own.to_lowercase();
    let signals: Vec<_> = input
        .classification_signals
        .lines()
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    let list = signals.iter().any(|s| {
        matches!(
            s.as_str(),
            "mailing-list identifier header present"
                | "mailing-list precedence header"
                | "bulk-mail precedence header"
        )
    });
    let unsubscribe = signals
        .iter()
        .any(|s| s == "mailing-list unsubscribe header present");
    let generated = signals.iter().any(|s| {
        matches!(
            s.as_str(),
            "automatically generated message header" | "automatically replied message header"
        )
    });
    for (kind, phrases) in [
        (
            DetectedType::Receipt,
            &[
                "payment receipt",
                "your receipt",
                "payment received",
                "payment was received",
            ][..],
        ),
        (
            DetectedType::Invoice,
            &[
                "your invoice",
                "invoice attached",
                "invoice number",
                "invoice #",
            ],
        ),
        (
            DetectedType::Order,
            &[
                "order confirmation",
                "your order #",
                "your order number",
                "thanks for your order",
                "thank you for your order",
            ],
        ),
        (
            DetectedType::Shipping,
            &[
                "your shipment",
                "your package",
                "your parcel",
                "has shipped",
                "has been shipped",
                "out for delivery",
                "delivery confirmation",
            ],
        ),
        (
            DetectedType::EventInvitation,
            &[
                "you have been invited",
                "invitation:",
                "event invitation",
                "meeting invitation",
            ],
        ),
        (
            DetectedType::VerificationCode,
            &[
                "verification code",
                "security code",
                "one-time code",
                "one time code",
                "login code",
                "sign-in code",
                "authentication code",
                "pin code",
                "one-time password",
                "your otp",
            ],
        ),
        (
            DetectedType::FlightBooking,
            &[
                "flight booking",
                "flight confirmation",
                "flight itinerary",
                "your flight",
                "flight cancelled",
                "flight canceled",
                "flight cancellation",
                "flight delayed",
                "flight rescheduled",
            ],
        ),
        (
            DetectedType::Newsletter,
            &[
                "your weekly newsletter",
                "this week's newsletter",
                "newsletter:",
                "weekly digest",
            ],
        ),
        (
            DetectedType::Offer,
            &[
                "exclusive offer",
                "shop now",
                "discount code",
                "sale ends",
                "special offer",
            ],
        ),
        (
            DetectedType::SecurityAlert,
            &[
                "new sign-in",
                "new sign in",
                "suspicious activity",
                "unrecognized login",
                "password was changed",
                "password has been changed",
            ],
        ),
    ] {
        if has(&text, phrases) {
            add(&mut out, kind);
        }
    }
    if list || (unsubscribe && !input.known_correspondence) {
        add(&mut out, DetectedType::Newsletter);
    }
    static SCRIPT: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse("script[type='application/ld+json']").unwrap());
    for script in doc.select(&SCRIPT).take(8) {
        if let Ok(value) = serde_json::from_str::<Value>(&script.inner_html()) {
            let context = value.get("@context").and_then(Value::as_str).unwrap_or("");
            if matches!(
                context,
                "http://schema.org" | "https://schema.org" | "https://schema.org/"
            ) {
                structured(&value, &mut out, 0);
            }
        }
    }
    calendar(&input.calendar, &mut out);
    let payment_failure = has(
        &text,
        &[
            "payment failed",
            "payment failure",
            "payment declined",
            "couldn't process your payment",
            "could not process your payment",
        ],
    );
    let machine_transaction = out.types.iter().any(|t| {
        matches!(
            t,
            DetectedType::Receipt
                | DetectedType::Order
                | DetectedType::Shipping
                | DetectedType::VerificationCode
                | DetectedType::FlightBooking
        )
    }) || payment_failure;
    let direct_reply = input.known_correspondence
        && !generated
        && !list
        && (!machine_transaction || input.subject.to_lowercase().starts_with("re:"));
    let personal_invitation = input.known_correspondence
        && out.types.contains(&DetectedType::EventInvitation)
        && !machine_transaction
        && !list
        && !unsubscribe
        && !out.types.contains(&DetectedType::Offer);
    if !direct_reply
        && !personal_invitation
        && (generated
            || list
            || unsubscribe
            || out
                .types
                .iter()
                .any(|t| !matches!(t, DetectedType::EventInvitation | DetectedType::Invoice))
            || payment_failure)
    {
        out.kind = InboxKind::Automated;
    }
    if out.types.contains(&DetectedType::FlightBooking) {
        if let Some(number) = regex(
            r"(?i)flight\s*(?:number|no\.?|#)?\s*([A-Z]{2}\s?\d{1,4})\b",
            &own,
        ) {
            detail(
                &mut out,
                DetectedType::FlightBooking,
                "Flight",
                number,
                None,
            );
        }
        if let Some(route) = regex(r"(?i)\bfrom\s+([A-Z]{3}\s+to\s+[A-Z]{3})\b", &own) {
            detail(
                &mut out,
                DetectedType::FlightBooking,
                "Route",
                route.to_uppercase(),
                None,
            );
        }
    }
    if out.types.contains(&DetectedType::EventInvitation) {
        if let Some(title) = input
            .subject
            .strip_prefix("Invitation:")
            .filter(|s| !s.trim().is_empty())
        {
            detail(
                &mut out,
                DetectedType::EventInvitation,
                "Event",
                title.trim().into(),
                None,
            );
        }
    }
    if out.types.contains(&DetectedType::VerificationCode) {
        // Require one contextual candidate; dates and unrelated bare numbers are not codes.
        let pattern = r"(?i)(?:verification code|security code|one[- ]time code|login code|sign-in code|authentication code|pin code|one[- ]time password|otp)\s*(?:is|:|=)?\s*([A-Z0-9]{4,10})\b";
        let re = Regex::new(pattern).unwrap();
        let candidates: Vec<_> = re
            .captures_iter(&own)
            .filter_map(|c| c.get(1))
            .map(|c| c.as_str())
            .filter(|s| s.chars().any(|c| c.is_ascii_digit()))
            .collect();
        if candidates.len() == 1 {
            detail(
                &mut out,
                DetectedType::VerificationCode,
                "Code",
                candidates[0].into(),
                Some(DetailAction::Copy),
            );
            if let Ok(received) = DateTime::parse_from_rfc3339(&input.received_at) {
                let validity = Regex::new(
                    r"(?i)(?:expires? in|valid for)\s+(\d{1,3})\s+(seconds?|minutes?|hours?)",
                )
                .unwrap();
                let seconds = validity
                    .captures(&own)
                    .and_then(|c| {
                        let n = c.get(1)?.as_str().parse::<i64>().ok()?;
                        let unit = c.get(2)?.as_str().to_lowercase();
                        Some(
                            n * if unit.starts_with("hour") {
                                3600
                            } else if unit.starts_with("minute") {
                                60
                            } else {
                                1
                            },
                        )
                    })
                    .filter(|n| *n > 0 && *n <= 86400)
                    .unwrap_or(900);
                attention(
                    &mut out,
                    "Verification code",
                    Some(
                        (received + Duration::seconds(seconds))
                            .to_rfc3339_opts(SecondsFormat::Millis, true),
                    ),
                );
            }
        }
    }
    if payment_failure {
        attention(&mut out, "Payment failed", None);
    }
    if out.types.contains(&DetectedType::FlightBooking)
        && has(
            &text,
            &[
                "cancelled",
                "canceled",
                "delayed",
                "rescheduled",
                "schedule change",
            ],
        )
    {
        attention(&mut out, "Flight changed", None);
    }
    if out.types.contains(&DetectedType::EventInvitation)
        && has(&text, &["cancelled", "canceled", "rescheduled"])
    {
        attention(&mut out, "Event changed", None);
    }
    if out.types.contains(&DetectedType::SecurityAlert) {
        attention(&mut out, "Account security", None);
    }
    if let Some(reference) = regex(
        r"(?i)(?:order (?:number|reference|#)|booking reference|tracking number)\s*[:#]?\s*([A-Z0-9][A-Z0-9-]{3,24})\b",
        &own,
    ) {
        let t = if out.types.contains(&DetectedType::FlightBooking) {
            DetectedType::FlightBooking
        } else if out.types.contains(&DetectedType::Shipping) {
            DetectedType::Shipping
        } else {
            DetectedType::Order
        };
        if out.types.contains(&t) {
            detail(
                &mut out,
                t,
                "Reference",
                reference,
                Some(DetailAction::Copy),
            );
        }
    }
    if out.types.iter().any(|t| {
        matches!(
            t,
            DetectedType::Receipt | DetectedType::Invoice | DetectedType::Order
        )
    }) {
        if let Some(amount) = regex(
            r"(?i)(?:total(?: paid| due)?|amount paid|amount due|payment of)\s*:?\s*((?:USD|EUR|GBP|\$|€|£)\s*\d[\d,]*(?:\.\d{2})?|\d[\d,]*(?:\.\d{2})?\s*(?:USD|EUR|GBP))\b",
            &own,
        ) {
            let t = if out.types.contains(&DetectedType::Receipt) {
                DetectedType::Receipt
            } else if out.types.contains(&DetectedType::Invoice) {
                DetectedType::Invoice
            } else {
                DetectedType::Order
            };
            detail(&mut out, t, "Total", amount, None);
        }
    }
    for (t, phrases, label) in [
        (
            DetectedType::Shipping,
            &["out for delivery", "delivered", "in transit", "has shipped"][..],
            "Status",
        ),
        (
            DetectedType::FlightBooking,
            &["cancelled", "canceled", "delayed", "rescheduled"][..],
            "Status",
        ),
    ] {
        if out.types.contains(&t) {
            if let Some(p) = phrases.iter().find(|p| text.contains(**p)) {
                detail(&mut out, t, label, (*p).into(), None);
            }
        }
    }
    static ANCHOR: LazyLock<Selector> = LazyLock::new(|| Selector::parse("a[href]").unwrap());
    for a in doc.select(&ANCHOR).take(100) {
        let label = a.text().collect::<Vec<_>>().join(" ").to_lowercase();
        let link = a.value().attr("href").unwrap_or("");
        if !safe_link(link) {
            continue;
        }
        if out.types.contains(&DetectedType::Shipping)
            && has(
                &label,
                &["track shipment", "track package", "track your order"],
            )
        {
            detail(
                &mut out,
                DetectedType::Shipping,
                "Track shipment",
                link.into(),
                Some(DetailAction::Open),
            );
        }
        if out.types.contains(&DetectedType::FlightBooking)
            && has(&label, &["manage booking", "manage your booking"])
        {
            detail(
                &mut out,
                DetectedType::FlightBooking,
                "Manage booking",
                link.into(),
                Some(DetailAction::Open),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mail(subject: &str, body: &str) -> EmailClassificationInput {
        EmailClassificationInput::new(Some("Alex"), "alex@example.test", subject, body, "")
    }
    #[test]
    fn unfamiliar_correspondence_is_personal() {
        for subject in [
            "Friday planning",
            "Hallo, wie geht es dir?",
            "Re: a quick question",
        ] {
            assert_eq!(
                detect(&mail(subject, "Could we talk tomorrow?")).kind,
                InboxKind::Personal
            );
        }
        assert_eq!(
            detect(&EmailClassificationInput::new(
                Some("Service"),
                "no-reply@service.test",
                "A note",
                "Hello",
                ""
            ))
            .kind,
            InboxKind::Personal
        );
    }
    #[test]
    fn overlapping_types_and_context_are_independent() {
        let mut m = mail(
            "Your order confirmation and payment receipt",
            "Your shipment has shipped. Total: EUR 42.00",
        );
        m.known_correspondence = true;
        let c = detect(&m);
        assert_eq!(c.kind, InboxKind::Automated);
        for t in [
            DetectedType::Order,
            DetectedType::Receipt,
            DetectedType::Shipping,
        ] {
            assert!(c.types.contains(&t));
        }
        assert!(c.details.iter().any(|d| d.value == "EUR 42.00"));
        let c = detect(
            &mail("Invitation: lunch", "You have been invited").with_known_correspondence(true),
        );
        assert_eq!(c.kind, InboxKind::Personal);
        assert!(c.types.contains(&DetectedType::EventInvitation));
    }
    #[test]
    fn mixed_support_and_named_newsletters() {
        assert_eq!(
            detect(
                &mail("Re: your invoice", "I can help with that").with_known_correspondence(true)
            )
            .kind,
            InboxKind::Personal
        );
        let mut m = mail("A note from Alex", "Hello");
        m.classification_signals = "Mailing-list identifier header present".into();
        m.known_correspondence = true;
        assert_eq!(detect(&m).kind, InboxKind::Automated);
        let mut m = mail("Account settings updated", "Your preferences changed");
        m.classification_signals = "Automatically generated message header".into();
        let c = detect(&m);
        assert_eq!(c.kind, InboxKind::Automated);
        assert!(c.types.is_empty());
    }
    #[test]
    fn codes_need_context_and_expire_without_using_wall_clock() {
        let mut m = mail("Your verification code: 123456", "Valid for 5 minutes");
        m.received_at = "2026-11-12T10:00:00.000Z".into();
        let c = detect(&m);
        assert!(c.needs_attention("2026-11-12T10:04:00.000Z"));
        assert!(!c.needs_attention("2026-11-12T10:05:00.000Z"));
        assert!(detect(&mail("Verification code", "Order 123456 from 2026"))
            .details
            .is_empty());
        assert!(detect(&mail(
            "Verification code: 123456",
            "Verification code: 654321"
        ))
        .details
        .is_empty());
    }
    #[test]
    fn html_quotes_and_untrusted_signals_do_not_create_attention() {
        let mut m = mail("Hello", "");
        m.html="<p>Let's talk.</p><blockquote>Your verification code: 123456</blockquote><script>payment failed</script>".into();
        assert_eq!(detect(&m).kind, InboxKind::Personal);
        assert_eq!(
            detect(&mail("Hello", "Automatically generated message header")).kind,
            InboxKind::Personal
        );
        assert!(
            detect(&mail("Re: code", "Thanks\n> Verification code: 123456"))
                .attention
                .is_empty()
        );
    }
    #[test]
    fn structured_flights_and_safe_links() {
        let mut m = mail("Your trip", "");
        m.html=r#"<script type="application/ld+json">{"@context":"https://schema.org","@type":"FlightReservation","reservationNumber":"ABC123","reservationStatus":"https://schema.org/ReservationCancelled"}</script><a href="https://airline.example/manage">Manage booking</a>"#.into();
        let c = detect(&m);
        assert!(c.types.contains(&DetectedType::FlightBooking));
        assert!(c.needs_attention("2026-11-12"));
        assert!(c
            .details
            .iter()
            .any(|d| d.action == Some(DetailAction::Open)));
        for url in [
            "javascript:alert(1)",
            "https://user:pass@host.test",
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://localhost/",
            "file:///tmp/a",
        ] {
            assert!(!safe_link(url), "{url}");
        }
    }
    #[test]
    fn urgency_is_not_attention() {
        assert!(detect(&mail("URGENT special offer", "Sale ends tonight"))
            .attention
            .is_empty());
        assert!(
            detect(&mail("Your payment failed", "Update your payment details"))
                .needs_attention("2026-11-12")
        );
    }
}
