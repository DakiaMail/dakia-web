//! Owned candidate predicates shared by the local accelerator and SQL retrieval.
use crate::{normalize_search, CachedSearchFilter, Message};
#[derive(Clone)]
pub(crate) struct CandidateMetadata {
    pub message: Message,
    pub sender: String,
    pub recipients: String,
    pub subject: String,
    pub snoozed_until: Option<String>,
}
#[derive(Clone)]
pub(crate) struct CandidateFilter {
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub since: Option<String>,
    pub before: Option<String>,
    pub seen: Option<bool>,
    pub starred: Option<bool>,
    pub folders: Option<Vec<String>>,
    pub category: Option<String>,
    pub classification: dakia_domain::ClassificationFilter,
    pub snoozed_only: bool,
    pub hide_snoozed: bool,
    pub at: String,
}
impl From<&CachedSearchFilter<'_>> for CandidateFilter {
    fn from(f: &CachedSearchFilter<'_>) -> Self {
        Self {
            from: f.from.map(normalize_search),
            to: f.to.map(normalize_search),
            subject: f.subject.map(normalize_search),
            since: f.since.map(str::to_owned),
            before: f.before.map(str::to_owned),
            seen: f.seen,
            starred: f.starred,
            folders: f.folders.map(<[String]>::to_vec),
            category: f.category.map(str::to_owned),
            classification: f.classification.clone(),
            snoozed_only: f.snoozed_only,
            hide_snoozed: f.hide_snoozed,
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }
    }
}
impl CandidateFilter {
    pub fn restricted(&self) -> bool {
        self.from.is_some()
            || self.to.is_some()
            || self.subject.is_some()
            || self.since.is_some()
            || self.before.is_some()
            || self.seen.is_some()
            || self.starred.is_some()
            || self.folders.is_some()
            || self.category.is_some()
            || self.classification != dakia_domain::ClassificationFilter::default()
            || self.snoozed_only
            || self.hide_snoozed
    }
    pub fn matches(&self, m: &CandidateMetadata) -> bool {
        let row = &m.message;
        let snoozed = m
            .snoozed_until
            .as_deref()
            .is_some_and(|until| until > self.at.as_str());
        !row.folder_ids.is_empty()
            && [
                (self.from.as_deref(), m.sender.as_str()),
                (self.to.as_deref(), m.recipients.as_str()),
                (self.subject.as_deref(), m.subject.as_str()),
            ]
            .into_iter()
            .all(|(q, field)| q.is_none_or(|q| field.contains(q)))
            && self.classification.matches(row, &self.at)
            && self.seen.is_none_or(|seen| row.unread != seen)
            && self.starred.is_none_or(|starred| row.starred == starred)
            && self
                .folders
                .as_ref()
                .is_none_or(|fs| row.folder_ids.iter().any(|folder| fs.contains(folder)))
            && self.category.as_ref().is_none_or(|category| {
                row.category
                    .as_ref()
                    .is_none_or(|current| current == category)
            })
            && (self.since.is_none() && self.before.is_none()
                || row.received_date.as_deref().is_some_and(|day| {
                    self.since.as_deref().is_none_or(|since| day >= since)
                        && self.before.as_deref().is_none_or(|before| day < before)
                }))
            && (!self.snoozed_only || snoozed)
            && (!self.hide_snoozed || !snoozed)
    }
}

/// A three-character literal prefix bounds FTS query size even for very long
/// values. The caller must retain its complete literal substring predicate.
pub(super) fn sqlite_substring(column: &str, value: &str) -> Option<String> {
    // Intersect bounded, overlapping trigrams without phrase position work.
    // A three-character prefix such as "per" selects almost every participant;
    // a long native phrase also needlessly decodes positions for common grams.
    // Every true literal match contains these grams. SQL verifies the full value.
    let chars: Vec<_> = value.chars().take(64).collect();
    if chars.len() < 3 || chars.contains(&'\0') {
        return None;
    }
    let grams: std::collections::BTreeSet<String> =
        chars.windows(3).map(|gram| gram.iter().collect()).collect();
    Some(format!(
        "{column}:({})",
        grams
            .into_iter()
            .map(|gram| format!("\"{}\"", gram.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ")
    ))
}
pub(super) fn postgres_substring(value: &str) -> String {
    format!(
        "%{}%",
        value
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_")
    )
}

#[cfg(test)]
mod substring_tests {
    use super::*;
    #[test]
    fn candidates_escape_syntax_without_changing_literal_filters() {
        assert_eq!(
            sqlite_substring("subject", "\"éΩ"),
            Some("subject:(\"\"\"éΩ\")".into())
        );
        assert_eq!(
            sqlite_substring("subject", "abcd"),
            Some("subject:(\"abc\" AND \"bcd\")".into())
        );
        assert_eq!(sqlite_substring("sender", "東京"), None);
        assert_eq!(sqlite_substring("sender", "a\0b"), None);
        assert_eq!(
            sqlite_substring("sender", &"x".repeat(100)),
            Some("sender:(\"xxx\")".into())
        );
        assert_eq!(postgres_substring("%_!\\"), "%!%!_!!\\%");
    }
}
