//! Disposable USearch graphs. SQLite remains the authority for every vector.
//! A durable manifest commits snapshots before replay receipts can be pruned.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

type Key = (String, String);
pub(crate) type CohortVectors = Vec<(i64, String, Vec<u8>)>;
pub(crate) enum FilterCohort {
    Unavailable,
    Broad,
    Vectors(CohortVectors),
}
#[path = "search_vector_literals.rs"]
mod literals;
#[path = "search_vector_metadata.rs"]
mod metadata;
use metadata::{Metadata, Row};

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    format: u32,
    identity: String,
    watermark: i64,
    graphs: Vec<Snapshot>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    account: String,
    revision: String,
    file: String,
    sha256: String,
}
struct State {
    identity: String,
    watermark: i64,
    rebuild_cursor: Option<i64>,
    metadata_cursor: Option<i64>,
    metadata: Metadata,
    ready: bool,
    dirty: bool,
    vectors_dirty: bool,
    snapshots: Vec<Snapshot>,
    graphs: BTreeMap<Key, Index>,
}
pub(crate) struct VectorAccelerator {
    root: PathBuf,
    state: Arc<RwLock<Option<State>>>,
    turn: tokio::sync::Mutex<()>,
}
fn vector_error(_: impl std::fmt::Display) -> Error {
    Error::Invalid
}
fn index() -> Result<Index> {
    Index::new(&IndexOptions {
        dimensions: 384,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 128,
        ..Default::default()
    })
    .map_err(vector_error)
}
/// Offline packaged native-code check using caller-supplied synthetic vectors.
/// Exercise serialization too, without touching an application installation.
pub fn check_search_accelerator(query: &[f32], relevant: &[f32], distractor: &[f32]) -> Result<()> {
    if [query, relevant, distractor]
        .iter()
        .any(|v| v.len() != 384 || v.iter().any(|n| !n.is_finite()))
    {
        return Err(Error::Invalid);
    }
    let graph = index()?;
    graph.reserve(2).map_err(vector_error)?;
    graph.add(1, relevant).map_err(vector_error)?;
    graph.add(2, distractor).map_err(vector_error)?;
    let mut bytes = vec![0; graph.serialized_length()];
    graph.save_to_buffer(&mut bytes).map_err(vector_error)?;
    let restored = index()?;
    restored.load_from_buffer(&bytes).map_err(vector_error)?;
    let result = restored.search(query, 2).map_err(vector_error)?;
    if result.keys.first() != Some(&1) || result.distances.iter().any(|d| !d.is_finite()) {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn digest(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn load(root: &Path, identity: &str, floor: i64, high: i64) -> Result<State> {
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("snapshot.lock"))?;
    fs2::FileExt::lock_shared(&lock)?;
    let manifest: Manifest =
        serde_json::from_reader(std::fs::File::open(root.join("manifest.json"))?)?;
    if manifest.format != 1
        || manifest.identity != identity
        || manifest.watermark < floor
        || manifest.watermark > high
    {
        return Err(Error::Invalid);
    }
    let mut graphs = BTreeMap::new();
    let snapshots = manifest.graphs.clone();
    for snapshot in manifest.graphs {
        // Snapshot names are generated UUIDs, never account IDs or model paths.
        let name = snapshot
            .file
            .strip_suffix(".usearch")
            .ok_or(Error::Invalid)?;
        uuid::Uuid::parse_str(name).map_err(vector_error)?;
        let path = root.join(&snapshot.file);
        if digest(&path)? != snapshot.sha256 {
            return Err(Error::Invalid);
        }
        let graph = index()?;
        graph
            .load(path.to_str().ok_or(Error::Invalid)?)
            .map_err(vector_error)?;
        if graph.dimensions() != 384 {
            return Err(Error::Invalid);
        }
        if graphs
            .insert((snapshot.account, snapshot.revision), graph)
            .is_some()
        {
            return Err(Error::Invalid);
        }
    }
    Ok(State {
        identity: identity.to_owned(),
        watermark: manifest.watermark,
        rebuild_cursor: None,
        metadata_cursor: Some(0),
        metadata: Metadata::default(),
        ready: false,
        dirty: false,
        vectors_dirty: false,
        snapshots,
        graphs,
    })
}
fn apply(state: &mut State, changes: Vec<Row>, metadata_only: bool) -> Result<()> {
    for row in changes {
        state.metadata.apply(&row)?;
        if metadata_only {
            continue;
        }
        let key = u64::try_from(row.id).map_err(vector_error)?;
        let entry = (row.account, row.revision);
        if let Some(bytes) = row.bytes.filter(|_| row.valid) {
            if row.metadata_only
                && state
                    .graphs
                    .get(&entry)
                    .is_some_and(|graph| graph.contains(key))
            {
                continue;
            }
            state.vectors_dirty = true;
            if bytes.len() != 1536 {
                return Err(Error::Invalid);
            }
            let vector: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            if vector.iter().any(|n| !n.is_finite()) {
                return Err(Error::Invalid);
            }
            if !state.graphs.contains_key(&entry) {
                state.graphs.insert(entry.clone(), index()?);
            }
            let graph = state.graphs.get(&entry).ok_or(Error::Invalid)?;
            // Replay is idempotent, including replacement and delete receipts.
            if graph.contains(key) {
                graph.remove(key).map_err(vector_error)?;
            }
            if graph.size() == graph.capacity() {
                graph
                    .reserve((graph.size() + 1).next_power_of_two().max(64))
                    .map_err(vector_error)?;
            }
            graph.add(key, &vector).map_err(vector_error)?;
        } else if let Some(graph) = state.graphs.get(&entry) {
            state.vectors_dirty = true;
            graph.remove(key).map_err(vector_error)?;
            if graph.size() == 0 {
                state.graphs.remove(&entry);
            }
        }
    }
    state.dirty |= !metadata_only;
    Ok(())
}
impl VectorAccelerator {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.join("search/vectors"),
            state: Arc::new(RwLock::new(None)),
            turn: tokio::sync::Mutex::new(()),
        }
    }
    /// One bounded DB read and one bounded graph mutation. The initial build
    /// streams authoritative chunks; changes concurrent with it replay afterward.
    pub async fn advance(&self, db: &SqlitePool) -> Result<bool> {
        let _turn = self.turn.lock().await;
        // Reclaim acknowledged receipts in bounded transactions. Advancing the
        // floor is atomic with a durable snapshot, deletion can lag safely.
        let reclaimed=sqlx::query("DELETE FROM embedding_changes WHERE seq IN (SELECT seq FROM embedding_changes WHERE seq<=(SELECT floor FROM vector_replay WHERE id=1) ORDER BY seq LIMIT 4096)").execute(db).await?.rows_affected();
        let (identity,floor,high):(String,i64,i64)=sqlx::query_as("SELECT i.value,r.floor,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='embedding_changes'),0) FROM index_identity i CROSS JOIN vector_replay r WHERE i.id=1 AND r.id=1").fetch_one(db).await?;
        uuid::Uuid::parse_str(&identity).map_err(vector_error)?;
        let reset = {
            let state = self.state.read().map_err(vector_error)?;
            state
                .as_ref()
                .is_none_or(|s| s.identity != identity || s.watermark < floor || s.watermark > high)
        };
        if reset {
            let root = self.root.join(&identity);
            let state = self.state.clone();
            let identity = identity.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let loaded = load(&root, &identity, floor, high).unwrap_or_else(|_| State {
                    identity,
                    watermark: high,
                    rebuild_cursor: Some(0),
                    metadata_cursor: None,
                    metadata: Metadata::default(),
                    ready: false,
                    dirty: true,
                    vectors_dirty: true,
                    snapshots: vec![],
                    graphs: BTreeMap::new(),
                });
                *state.write().map_err(vector_error)? = Some(loaded);
                Ok(())
            })
            .await
            .map_err(vector_error)??;
        }
        let (watermark, cursor, metadata_cursor) = {
            let state = self.state.read().map_err(vector_error)?;
            let s = state.as_ref().ok_or(Error::Invalid)?;
            (s.watermark, s.rebuild_cursor, s.metadata_cursor)
        };
        let building = cursor.is_some();
        let metadata_only = !building && metadata_cursor.is_some();
        let changes = metadata::rows(
            db,
            cursor.or(metadata_cursor).unwrap_or(watermark),
            !building && !metadata_only,
        )
        .await?;
        let next = changes.last().map(|row| row.cursor);
        let worked = !changes.is_empty();
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut guard = state.write().map_err(vector_error)?;
            let s = guard.as_mut().ok_or(Error::Invalid)?;
            if worked {
                apply(s, changes, metadata_only)?;
            }
            if building {
                s.rebuild_cursor = next;
            } else if metadata_only {
                s.metadata_cursor = next;
            } else if let Some(next) = next {
                s.watermark = next;
            }
            if !building && !metadata_only && !worked {
                s.ready = true;
            }
            Ok(())
        })
        .await
        .map_err(vector_error)??;
        Ok(worked || building || metadata_only || reclaimed > 0)
    }
    /// Search never opens or rebuilds a graph on the request path. A caller can
    /// retain lexical/provider results while this component catches up.
    #[cfg(test)]
    pub async fn candidates(
        &self,
        account: &str,
        revision: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<(i64, f32)>> {
        self.filtered_candidates(account, revision, vector, limit, None)
            .await
    }
    #[cfg(test)]
    pub async fn filtered_candidates(
        &self,
        account: &str,
        revision: &str,
        vector: &[f32],
        limit: usize,
        filter: Option<crate::search_backend::filter::CandidateFilter>,
    ) -> Result<Vec<(i64, f32)>> {
        self.search(account, revision, vector, limit, filter, None)
            .await
    }
    /// Resident metadata supplies identities only. The caller still verifies
    /// target payload versions, the replay frontier and live application state.
    pub async fn message_candidates(
        &self,
        account: &str,
        revision: &str,
        vector: &[f32],
        filter: crate::search_backend::filter::CandidateFilter,
    ) -> Result<Vec<(i64, f32)>> {
        self.search(account, revision, vector, 1024, Some(filter), Some(200))
            .await
    }
    /// Resolve a strict filter cohort using replayed normalized-value indexes. SQL
    /// fallback remains available until every graph and metadata page is ready.
    pub async fn filter_cohort_vectors(
        &self,
        account: &str,
        revision: &str,
        filter: crate::search_backend::filter::CandidateFilter,
    ) -> Result<FilterCohort> {
        if filter.from.is_none()
            && filter.to.is_none()
            && filter.subject.is_none()
            && filter.seen.is_none()
            && filter.starred.is_none()
            && filter.since.is_none()
            && filter.before.is_none()
        {
            return Ok(FilterCohort::Unavailable);
        }
        let state = self.state.clone();
        let identity = (account.to_owned(), revision.to_owned());
        tokio::task::spawn_blocking(move || -> Result<FilterCohort> {
            let guard = state.try_read().map_err(vector_error)?;
            let Some(state) = guard.as_ref().filter(|state| state.ready) else {
                return Ok(FilterCohort::Unavailable);
            };
            let Some(graph) = state.graphs.get(&identity) else {
                return Ok(FilterCohort::Unavailable);
            };
            let Some(ids) = state.metadata.literals.cohort(&identity.0, &filter, |id| {
                graph.contains(id)
                    && state
                        .metadata
                        .entries
                        .get(&id)
                        .is_some_and(|metadata| filter.matches(metadata))
            }) else {
                return Ok(FilterCohort::Broad);
            };
            let mut vectors = Vec::with_capacity(ids.len());
            let mut buffer = [0f32; 384];
            for id in ids {
                let metadata = state.metadata.entries.get(&id).ok_or(Error::Invalid)?;
                if graph.get(id, &mut buffer).map_err(vector_error)? != 1 {
                    return Ok(FilterCohort::Unavailable);
                }
                vectors.push((
                    i64::try_from(id).map_err(vector_error)?,
                    metadata.message.id.clone(),
                    buffer
                        .iter()
                        .flat_map(|value| value.to_le_bytes())
                        .collect(),
                ));
            }
            Ok(FilterCohort::Vectors(vectors))
        })
        .await
        .map_err(vector_error)?
    }
    /// An optional exact-cohort read from the lossless F32 accelerator. Callers
    /// fence both sides against the durable replay frontier and retain SQL as
    /// the authority when any graph/key is unavailable.
    pub async fn cohort_vectors(
        &self,
        account: &str,
        revision: &str,
        ids: &[i64],
    ) -> Result<Option<CohortVectors>> {
        if ids.len() > 4096 {
            return Err(Error::Invalid);
        }
        let state = self.state.clone();
        let identity = (account.to_owned(), revision.to_owned());
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || -> Result<Option<CohortVectors>> {
            let guard = state.try_read().map_err(vector_error)?;
            let Some(state) = guard.as_ref().filter(|state| state.ready) else {
                return Ok(None);
            };
            let Some(graph) = state.graphs.get(&identity) else {
                return Ok(None);
            };
            let mut vectors = Vec::with_capacity(ids.len());
            let mut buffer = [0f32; 384];
            for id in ids {
                let key = u64::try_from(id).map_err(vector_error)?;
                let Some(metadata) = state
                    .metadata
                    .entries
                    .get(&key)
                    .filter(|metadata| metadata.message.account_id == identity.0)
                else {
                    return Ok(None);
                };
                if graph.get(key, &mut buffer).map_err(vector_error)? != 1 {
                    return Ok(None);
                }
                vectors.push((
                    id,
                    metadata.message.id.clone(),
                    buffer
                        .iter()
                        .flat_map(|value| value.to_le_bytes())
                        .collect(),
                ));
            }
            Ok(Some(vectors))
        })
        .await
        .map_err(vector_error)?
    }
    async fn search(
        &self,
        account: &str,
        revision: &str,
        vector: &[f32],
        limit: usize,
        filter: Option<crate::search_backend::filter::CandidateFilter>,
        message_limit: Option<usize>,
    ) -> Result<Vec<(i64, f32)>> {
        if vector.len() != 384
            || vector.iter().any(|v| !v.is_finite())
            || !(1..=1024).contains(&limit)
        {
            return Err(Error::Invalid);
        }
        let state = self.state.clone();
        let key = (account.to_owned(), revision.to_owned());
        let vector = vector.to_vec();
        let (cancelled, cancellation) = tokio::sync::oneshot::channel::<()>();
        let result = tokio::task::spawn_blocking(move || -> Result<Vec<(i64, f32)>> {
            let guard = state.try_read().map_err(|_| Error::Conflict)?;
            let state = guard.as_ref().filter(|s| s.ready).ok_or(Error::Conflict)?;
            let Some(graph) = state.graphs.get(&key) else {
                return Ok(vec![]);
            };
            if graph.size() == 0 {
                return Ok(vec![]);
            }
            let accepted = |key| {
                state.metadata.entries.get(&key).is_some_and(|metadata| {
                    filter
                        .as_ref()
                        .is_none_or(|filter| filter.matches(metadata))
                })
            };
            // Small unrestricted graphs fit in the same bounded exact-work
            // envelope as SQL filter cohorts. This also avoids approximate
            // recall collapse around many identical chunks in a short mailbox.
            let small = graph.size() <= 4096 && filter.as_ref().is_none_or(|f| !f.restricted());
            let mut window = if message_limit.is_some() {
                limit.min(256)
            } else {
                limit
            };
            let exact_messages = |head: usize| -> Result<Vec<(i64, f32)>> {
                // Keep only the best passage per message, with bounded scratch space.
                let mut best = std::collections::HashMap::new();
                let mut ordered = std::collections::BTreeSet::new();
                let qnorm = vector
                    .iter()
                    .map(|v| f64::from(*v).powi(2))
                    .sum::<f64>()
                    .sqrt();
                let mut buffer = [0f32; 384];
                for (&id, metadata) in &state.metadata.entries {
                    if cancelled.is_closed() {
                        return Err(Error::Conflict);
                    }
                    if metadata.message.account_id != key.0 || !accepted(id) || !graph.contains(id)
                    {
                        continue;
                    }
                    if graph.get(id, &mut buffer).map_err(vector_error)? != 1 {
                        return Err(Error::Conflict);
                    }
                    let mut dot = 0.;
                    let mut norm = 0.;
                    for (v, q) in buffer.iter().zip(&vector) {
                        dot += f64::from(*v) * f64::from(*q);
                        norm += f64::from(*v).powi(2);
                    }
                    if norm <= 0. || !norm.is_finite() {
                        return Err(Error::Invalid);
                    }
                    let distance =
                        ((1. - dot / (qnorm * norm.sqrt())).clamp(0., 2.) as f32).to_bits();
                    let message = metadata.message.id.as_str();
                    let hit = (distance, id, message);
                    if let Some(old) = best.get(message) {
                        if *old <= hit {
                            continue;
                        }
                        ordered.remove(old);
                    }
                    best.insert(message, hit);
                    ordered.insert(hit);
                    if ordered.len() > head {
                        let worst = ordered.pop_last().unwrap();
                        best.remove(worst.2);
                    }
                }
                ordered
                    .into_iter()
                    .map(|(distance, id, _)| {
                        Ok((
                            i64::try_from(id).map_err(vector_error)?,
                            f32::from_bits(distance),
                        ))
                    })
                    .collect()
            };
            if let Some(head) =
                message_limit.filter(|_| filter.as_ref().is_some_and(|f| f.restricted()))
            {
                return exact_messages(head);
            }
            loop {
                let result = if small {
                    graph.exact_search(&vector, graph.size())
                } else {
                    graph.filtered_search(&vector, window, accepted)
                }
                .map_err(vector_error)?;
                let mut seen = std::collections::HashSet::new();
                let mut hits = Vec::new();
                for (key, distance) in result
                    .keys
                    .into_iter()
                    .zip(result.distances)
                    .filter(|(key, _)| !small || accepted(*key))
                    .take(window)
                {
                    if message_limit.is_some() {
                        let Some(metadata) = state.metadata.entries.get(&key) else {
                            continue;
                        };
                        if !seen.insert(metadata.message.id.as_str()) {
                            continue;
                        }
                    }
                    hits.push((i64::try_from(key).map_err(vector_error)?, distance));
                    if message_limit.is_some_and(|head| hits.len() == head) {
                        break;
                    }
                }
                // Short approximate pages do not establish exhaustion. Exact
                // small graphs can establish it; otherwise retain the ceiling.
                if message_limit.is_none()
                    || message_limit.is_some_and(|head| hits.len() == head)
                    || window == limit
                    || small && window >= graph.size()
                {
                    if let Some(head) = message_limit.filter(|head| hits.len() < *head) {
                        return exact_messages(head);
                    }
                    return Ok(hits);
                }
                window = (window * 2).min(limit);
            }
        })
        .await
        .map_err(vector_error)?;
        drop(cancellation);
        result
    }
    /// Atomic manifest replacement is the only commit point. A partial write
    /// leaves the preceding snapshot usable, with durable SQLite changes replayed.
    pub async fn snapshot(&self, db: &SqlitePool) -> Result<()> {
        let _turn = self.turn.lock().await;
        let root = self.root.clone();
        let state = self.state.clone();
        let watermark =
            tokio::task::spawn_blocking(move || -> Result<Option<(i64, String, Vec<Snapshot>)>> {
                let guard = state.read().map_err(vector_error)?;
                let s = guard.as_ref().filter(|s| s.ready).ok_or(Error::Conflict)?;
                if !s.dirty {
                    return Ok(None);
                }
                let root = root.join(&s.identity);
                std::fs::create_dir_all(&root)?;
                let lock = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(root.join("snapshot.lock"))?;
                fs2::FileExt::lock_exclusive(&lock)?;
                let mut manifest = Manifest {
                    format: 1,
                    identity: s.identity.clone(),
                    watermark: s.watermark,
                    graphs: vec![],
                };
                if !s.vectors_dirty
                    && s.snapshots
                        .iter()
                        .all(|snapshot| root.join(&snapshot.file).is_file())
                {
                    manifest.graphs = s.snapshots.clone();
                } else {
                    for ((account, revision), graph) in &s.graphs {
                        if graph.size() == 0 {
                            continue;
                        }
                        let file = format!("{}.usearch", uuid::Uuid::new_v4());
                        let path = root.join(&file);
                        graph
                            .save(path.to_str().ok_or(Error::Invalid)?)
                            .map_err(vector_error)?;
                        std::fs::File::open(&path)?.sync_all()?;
                        manifest.graphs.push(Snapshot {
                            account: account.clone(),
                            revision: revision.clone(),
                            sha256: digest(&path)?,
                            file,
                        });
                    }
                }
                let temporary = root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
                let mut output = std::fs::File::create(&temporary)?;
                serde_json::to_writer(&mut output, &manifest)?;
                output.sync_all()?;
                std::fs::rename(&temporary, root.join("manifest.json"))?;
                std::fs::File::open(&root)?.sync_all()?;
                // Remove only accelerator-owned obsolete files after committing.
                for entry in std::fs::read_dir(&root)? {
                    let path = entry?.path();
                    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    if (name.ends_with(".usearch") || name.ends_with(".tmp"))
                        && !manifest.graphs.iter().any(|g| g.file == name)
                    {
                        let _ = std::fs::remove_file(path);
                    }
                }
                Ok(Some((s.watermark, s.identity.clone(), manifest.graphs)))
            })
            .await
            .map_err(vector_error)??;
        if let Some((watermark, identity, snapshots)) = watermark {
            let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
            // A different process may already have committed a newer snapshot.
            let acknowledged=sqlx::query("UPDATE vector_replay SET floor=max(floor,?) WHERE id=1 AND EXISTS(SELECT 1 FROM index_identity WHERE id=1 AND value=?)")
                .bind(watermark).bind(&identity).execute(&mut *tx).await?.rows_affected();
            if acknowledged != 1 {
                return Err(Error::Conflict);
            }
            sqlx::query("DELETE FROM embedding_changes WHERE seq IN (SELECT seq FROM embedding_changes WHERE seq<=? ORDER BY seq LIMIT 4096)")
                .bind(watermark)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            if let Some(s) = self.state.write().map_err(vector_error)?.as_mut() {
                s.dirty = false;
                s.vectors_dirty = false;
                s.snapshots = snapshots;
            }
        }
        Ok(())
    }
}
impl VectorAccelerator {
    /// Durable completion and ANN publication are separate stages. A ready graph
    /// behind the replay frontier cannot establish complete semantic coverage.
    pub async fn caught_up(&self, db: &SqlitePool, account: &str, revision: &str) -> Result<bool> {
        let (identity, watermark) = {
            let state = self.state.try_read().map_err(|_| Error::Conflict)?;
            let Some(state) = state.as_ref().filter(|s| s.ready) else {
                return Ok(false);
            };
            (state.identity.clone(), state.watermark)
        };
        let (current,floor,high,pending):(String,i64,i64,bool)=sqlx::query_as("SELECT i.value,r.floor,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='embedding_changes'),0),EXISTS(SELECT 1 FROM embedding_changes WHERE account_id=?1 AND model_revision=?2 AND seq>?3) FROM index_identity i CROSS JOIN vector_replay r WHERE i.id=1 AND r.id=1").bind(account).bind(revision).bind(watermark).fetch_one(db).await?;
        Ok(current == identity && watermark >= floor && watermark <= high && !pending)
    }
}
