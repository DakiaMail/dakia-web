use anyhow::{ensure, Context, Result};
use dakia_domain::DetectedType;
use regex::Regex;
use serde::{Deserialize, Deserializer};
use std::{collections::BTreeMap, sync::LazyLock};

const SOURCE: &str = include_str!("../rules/classification.json");
pub(crate) static RULES: LazyLock<Rules> =
    LazyLock::new(|| Rules::parse(SOURCE).expect("valid bundled classification rules"));

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rules {
    format_version: u8,
    revision: String,
    pub type_phrases: Vec<TypeRule>,
    pub phrases: BTreeMap<String, Vec<String>>,
    #[serde(deserialize_with = "compile_patterns")]
    patterns: BTreeMap<String, Regex>,
    pub delivery_headers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TypeRule {
    #[serde(rename = "type")]
    pub email_type: DetectedType,
    pub any: Vec<String>,
}

fn compile_patterns<'de, D: Deserializer<'de>>(
    input: D,
) -> std::result::Result<BTreeMap<String, Regex>, D::Error> {
    BTreeMap::<String, String>::deserialize(input)?
        .into_iter()
        .map(|(name, pattern)| {
            Regex::new(&pattern)
                .map(|regex| (name.clone(), regex))
                .map_err(|error| serde::de::Error::custom(format!("pattern {name}: {error}")))
        })
        .collect()
}

impl Rules {
    fn parse(source: &str) -> Result<Self> {
        let rules: Self = serde_json::from_str(source).context("parse classification rules")?;
        ensure!(
            rules.format_version == 1,
            "unsupported classification rules format"
        );
        ensure!(!rules.revision.trim().is_empty(), "missing rules revision");
        for key in [
            "automated_sender",
            "automated_sender_address",
            "receipt_subject",
            "delivery_role_sender",
            "flight_number",
            "flight_route",
            "verification_code",
            "code_validity",
            "reference",
            "amount",
            "valid_amount",
            "event_title",
            "document_mailer",
            "reply_subject",
            "unsubscribe_path",
        ] {
            ensure!(rules.patterns.contains_key(key), "missing pattern {key}");
        }
        for (key, captures) in [
            ("flight_number", 1),
            ("flight_route", 1),
            ("verification_code", 1),
            ("code_validity", 2),
            ("reference", 1),
            ("amount", 1),
            ("event_title", 1),
        ] {
            ensure!(
                rules.patterns[key].captures_len() > captures,
                "missing capture group in {key}"
            );
        }
        ensure!(
            rules.patterns.values().all(|pattern| !pattern.is_match("")),
            "patterns must not match empty input"
        );
        for key in [
            "service_notice",
            "job_offer",
            "payment_failure",
            "invoice_subject",
            "flight_changed",
            "event_changed",
            "track_shipment",
            "manage_booking",
            "export_archive",
            "shipping_status",
            "flight_status",
        ] {
            ensure!(rules.phrases.contains_key(key), "missing phrase set {key}");
        }
        for phrases in rules
            .phrases
            .values()
            .chain(rules.type_phrases.iter().map(|rule| &rule.any))
        {
            ensure!(!phrases.is_empty() && phrases.iter().all(|phrase| !phrase.trim().is_empty() && phrase == &phrase.to_lowercase()), "phrase sets must contain nonempty lowercase text");
        }
        ensure!(!rules.type_phrases.is_empty(), "missing type rules");
        ensure!(
            !rules.delivery_headers.is_empty()
                && rules.delivery_headers.iter().all(|name| name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    && !name.is_empty()),
            "invalid delivery header names"
        );
        Ok(rules)
    }

    pub fn pattern(&self, name: &str) -> &Regex {
        &self.patterns[name]
    }

    pub fn has(&self, name: &str, text: &str) -> bool {
        self.phrases[name]
            .iter()
            .any(|phrase| text.contains(phrase))
    }
}

pub fn policy_revision() -> &'static str {
    &RULES.revision
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_rules_load_and_reject_broken_edits() {
        Rules::parse(SOURCE).unwrap();
        let original: serde_json::Value = serde_json::from_str(SOURCE).unwrap();
        for (field, value) in [
            ("format_version", serde_json::json!(2)),
            ("revision", serde_json::json!("")),
            (
                "type_phrases",
                serde_json::json!([{ "type": "unknown", "any": ["hello"] }]),
            ),
            ("patterns", serde_json::json!({ "automated_sender": "[" })),
            ("patterns", serde_json::json!({})),
            ("phrases", serde_json::json!({})),
            ("delivery_headers", serde_json::json!(["bad header"])),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut edited = original.clone();
            edited[field] = value;
            assert!(
                Rules::parse(&edited.to_string()).is_err(),
                "accepted invalid {field}"
            );
        }
        let mut edited = original;
        edited["phrases"]["service_notice"] = serde_json::json!([""]);
        assert!(Rules::parse(&edited.to_string()).is_err());
    }
}
