//! Bounded, coalesced literal-filter planning. A cached gram is always a
//! necessary part of the literal, even after frequencies or index identity change.
use super::*;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use tokio::sync::{Mutex, OnceCell};
type Entry = Arc<OnceCell<String>>;
#[derive(Default)]
struct Cache {
    entries: BTreeMap<String, Entry>,
    order: VecDeque<String>,
}
#[derive(Default)]
pub(super) struct Selectors {
    cache: Mutex<Cache>,
    #[cfg(test)]
    probes: std::sync::atomic::AtomicUsize,
}
impl Selectors {
    pub async fn literal(
        &self,
        db: &SqlitePool,
        column: &str,
        value: &str,
    ) -> Result<Option<String>> {
        let Some(fallback) = filter::sqlite_substring(column, value) else {
            return Ok(None);
        };
        let cell = {
            let mut cache = self.cache.lock().await;
            let cell = if let Some(cell) = cache.entries.get(&fallback) {
                cell.clone()
            } else {
                if cache.entries.len() == 128 {
                    if let Some(old) = cache.order.pop_front() {
                        cache.entries.remove(&old);
                    }
                }
                let cell = Arc::new(OnceCell::new());
                cache.entries.insert(fallback.clone(), cell.clone());
                cell
            };
            cache.order.retain(|old| old != &fallback);
            cache.order.push_back(fallback.clone());
            cell
        };
        Ok(Some(cell.get_or_try_init(|| async {
            #[cfg(test)]
            self.probes.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
            let chars: Vec<_> = value.chars().take(64).collect();
            let grams: std::collections::BTreeSet<String> = chars.windows(3).map(|gram| gram.iter().collect()).collect();
            let needles: Vec<_> = grams.into_iter().map(|gram| format!("{column}:\"{}\"",gram.replace('"',"\"\""))).collect();
            let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
            for (position,needle) in needles.iter().enumerate() {
                if position > 0 { q.push(" UNION ALL "); }
                q.push("SELECT ").push(position.to_string()).push(",COUNT(*) FROM (SELECT rowid FROM metadata_terms WHERE metadata_terms MATCH ").push_bind(needle).push(" LIMIT 4097) sample");
            }
            let counts: Vec<(i64,i64)> = q.build_query_as().fetch_all(db).await?;
            let selected = counts.into_iter().min_by_key(|(position,count)| (*count,*position)).ok_or(Error::Invalid)?;
            if selected.1 < 4097 {
                needles.get(usize::try_from(selected.0).map_err(|_| Error::Invalid)?).cloned().ok_or(Error::Invalid)
            } else { Ok(fallback.clone()) }
        }).await?.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn identical_planning_is_coalesced_and_the_cache_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap()
        else {
            unreachable!()
        };
        let selectors = Selectors::default();
        let queries = (0..8).map(|_| selectors.literal(&db, "sender", "person2@example.invalid"));
        let results = futures_util::future::join_all(queries).await;
        for result in results {
            assert_eq!(result.unwrap(), Some("sender:\".in\"".into()));
        }
        assert_eq!(
            selectors.probes.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        for n in 0..129 {
            selectors
                .literal(&db, "subject", &format!("distinct{n:04}"))
                .await
                .unwrap();
        }
        let cache = selectors.cache.lock().await;
        assert_eq!(cache.entries.len(), 128);
        assert_eq!(cache.order.len(), 128);
    }
}
