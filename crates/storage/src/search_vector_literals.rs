//! Disposable indexes over distinct normalized filter values. They select keys
//! only; replay fences, target versions and application authorization still apply.
use crate::search_backend::filter::{CandidateFilter, CandidateMetadata};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
pub(super) struct Literals {
    accounts: BTreeMap<String, [BTreeMap<String, BTreeSet<u64>>; 6]>,
}
fn values(metadata: &CandidateMetadata) -> [Option<&str>; 6] {
    [
        Some(&metadata.sender),
        Some(&metadata.recipients),
        Some(&metadata.subject),
        Some(if metadata.message.unread { "1" } else { "0" }),
        Some(if metadata.message.starred { "1" } else { "0" }),
        metadata.message.received_date.as_deref(),
    ]
}
impl Literals {
    pub fn insert(&mut self, id: u64, metadata: &CandidateMetadata) {
        let fields = self
            .accounts
            .entry(metadata.message.account_id.clone())
            .or_default();
        for (field, value) in fields.iter_mut().zip(values(metadata)) {
            if let Some(value) = value {
                field.entry(value.to_owned()).or_default().insert(id);
            }
        }
    }
    pub fn remove(&mut self, id: u64, metadata: &CandidateMetadata) {
        let account = &metadata.message.account_id;
        if let Some(fields) = self.accounts.get_mut(account) {
            for (field, value) in fields.iter_mut().zip(values(metadata)) {
                if let Some(value) = value {
                    if let Some(ids) = field.get_mut(value) {
                        ids.remove(&id);
                        if ids.is_empty() {
                            field.remove(value);
                        }
                    }
                }
            }
            if fields.iter().all(BTreeMap::is_empty) {
                self.accounts.remove(account);
            }
        }
    }
    /// None means the exact-work envelope is exceeded (or no indexed filter exists).
    /// Other predicates are checked before counting, so an intersecting strict
    /// filter never becomes broad merely because its first field is common.
    pub fn cohort(
        &self,
        account: &str,
        filter: &CandidateFilter,
        mut accept: impl FnMut(u64) -> bool,
    ) -> Option<Vec<u64>> {
        let Some(fields) = self.accounts.get(account) else {
            return Some(vec![]);
        };
        let mut choices: Vec<Vec<&BTreeSet<u64>>> = fields[..3]
            .iter()
            .zip([&filter.from, &filter.to, &filter.subject])
            .filter_map(|(field, value)| {
                value.as_deref().map(|value| {
                    field
                        .iter()
                        .filter_map(|(literal, ids)| literal.contains(value).then_some(ids))
                        .collect()
                })
            })
            .collect();
        for (field, value) in [(3, filter.seen.map(|seen| !seen)), (4, filter.starred)] {
            if let Some(value) = value {
                choices.push(
                    fields[field]
                        .get(if value { "1" } else { "0" })
                        .into_iter()
                        .collect(),
                );
            }
        }
        if filter.since.is_some() || filter.before.is_some() {
            choices.push(
                fields[5]
                    .iter()
                    .filter_map(|(day, ids)| {
                        (filter.since.as_ref().is_none_or(|since| day >= since)
                            && filter.before.as_ref().is_none_or(|before| day < before))
                        .then_some(ids)
                    })
                    .collect(),
            );
        }
        let selected = choices.into_iter().min_by_key(|buckets| {
            buckets
                .iter()
                .fold(0usize, |count, ids| count.saturating_add(ids.len()))
        })?;
        let mut ids = Vec::new();
        for bucket in selected {
            for &id in bucket {
                if accept(id) {
                    ids.push(id);
                    if ids.len() > 4096 {
                        return None;
                    }
                }
            }
        }
        Some(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata(account: &str, subject: &str) -> CandidateMetadata {
        let message=serde_json::from_value(serde_json::json!({"id":"indexed","account_id":account,"thread_id":"indexed","provider_id":"source","folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},"to":[],"subject":subject,"preview":"","received_at":"2026-01-01T00:00:00Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":1})).unwrap();
        CandidateMetadata {
            message,
            sender: "sender@example.invalid".into(),
            recipients: "carbon@example.invalid".into(),
            subject: subject.into(),
            snoozed_until: None,
        }
    }
    #[test]
    fn flag_and_internal_date_cohorts_keep_boundaries_and_replay_updates() {
        let mut index = Literals::default();
        let mut rows = BTreeMap::new();
        for (id, account, unread, starred, day) in [
            (1, "a", true, true, Some("2026-10-01")),
            (2, "a", true, false, Some("2026-10-14")),
            (3, "a", false, true, Some("2026-10-15")),
            (4, "a", false, false, None),
            (5, "b", true, true, Some("2026-10-01")),
        ] {
            let mut row = metadata(account, "subject");
            row.message.unread = unread;
            row.message.starred = starred;
            row.message.received_date = day.map(str::to_owned);
            index.insert(id, &row);
            rows.insert(id, row);
        }
        let cases = [
            (
                crate::CachedSearchFilter {
                    seen: Some(false),
                    ..Default::default()
                },
                vec![1, 2],
            ),
            (
                crate::CachedSearchFilter {
                    seen: Some(true),
                    ..Default::default()
                },
                vec![3, 4],
            ),
            (
                crate::CachedSearchFilter {
                    starred: Some(false),
                    ..Default::default()
                },
                vec![2, 4],
            ),
            (
                crate::CachedSearchFilter {
                    since: Some("2026-10-01"),
                    before: Some("2026-10-15"),
                    ..Default::default()
                },
                vec![1, 2],
            ),
            (
                crate::CachedSearchFilter {
                    starred: Some(true),
                    since: Some("2026-10-01"),
                    before: Some("2026-10-15"),
                    ..Default::default()
                },
                vec![1],
            ),
            (
                crate::CachedSearchFilter {
                    since: Some("2026-10-15"),
                    before: Some("2026-10-15"),
                    ..Default::default()
                },
                vec![],
            ),
        ];
        for (filter, expected) in cases {
            let filter = CandidateFilter::from(&filter);
            let mut actual = index
                .cohort("a", &filter, |id| filter.matches(&rows[&id]))
                .unwrap();
            actual.sort();
            assert_eq!(actual, expected);
        }
        let mut changed = rows[&1].clone();
        index.remove(1, &changed);
        changed.message.unread = false;
        changed.message.starred = false;
        changed.message.received_date = Some("2026-10-16".into());
        index.insert(1, &changed);
        rows.insert(1, changed);
        let filter = CandidateFilter::from(&crate::CachedSearchFilter {
            starred: Some(true),
            before: Some("2026-10-15"),
            ..Default::default()
        });
        assert_eq!(
            index.cohort("a", &filter, |id| filter.matches(&rows[&id])),
            Some(vec![])
        );
        assert_eq!(
            index.cohort("b", &filter, |id| filter.matches(&rows[&id])),
            Some(vec![5])
        );
        for (id, row) in rows {
            index.remove(id, &row);
        }
        assert!(index.accounts.is_empty());
    }
    #[test]
    fn selective_values_bound_keys_and_updates_remove_obsolete_buckets() {
        let mut index = Literals::default();
        let common = metadata("a", "ordinary");
        let rare = metadata("a", "rare subject");
        let other = metadata("b", "rare subject");
        for id in 1..=4097 {
            index.insert(id, &common);
        }
        index.insert(4098, &rare);
        index.insert(4099, &other);
        let mut filter = CandidateFilter::from(&crate::CachedSearchFilter {
            from: Some("sender"),
            ..Default::default()
        });
        assert!(index.cohort("a", &filter, |_| true).is_none());
        filter.subject = Some("rare".into());
        let mut visited = 0;
        assert_eq!(
            index.cohort("a", &filter, |_| {
                visited += 1;
                true
            }),
            Some(vec![4098])
        );
        assert_eq!(
            visited, 1,
            "other normalized-value buckets must not inspect message keys"
        );
        assert_eq!(index.cohort("b", &filter, |_| true), Some(vec![4099]));
        assert_eq!(index.cohort("a", &filter, |_| false), Some(vec![]));
        index.remove(4098, &rare);
        index.insert(4098, &common);
        assert_eq!(index.cohort("a", &filter, |_| true), Some(vec![]));
        for id in 1..=4098 {
            index.remove(id, &common);
        }
        index.remove(4099, &other);
        assert!(index.accounts.is_empty());
    }
}
