//! Slow-stage diagnostics contain static labels and elapsed time only.
pub(crate) struct Timing {
    at: std::time::Instant,
    stage: &'static str,
}
impl Timing {
    pub fn new(stage: &'static str) -> Self {
        Self {
            at: std::time::Instant::now(),
            stage,
        }
    }
    pub fn stage(&mut self, stage: &'static str) {
        self.report(true);
        self.at = std::time::Instant::now();
        self.stage = stage;
    }
    fn report(&self, finished: bool) {
        let elapsed_ms = self.at.elapsed().as_millis() as u64;
        if elapsed_ms >= 100 {
            tracing::info!(target: "dakia_server::provider_io", operation = "storage_wait", stage = self.stage, elapsed_ms, finished, "provider_io");
        }
    }
}
impl Drop for Timing {
    fn drop(&mut self) {
        self.report(false);
    }
}

/// Opt-in query-stage diagnostics. Only static stage names and elapsed time
/// enter logs; cancelled futures report the stage they were waiting for.
pub(crate) struct SearchTiming {
    at: std::time::Instant,
    stage: &'static str,
}
impl SearchTiming {
    pub fn new(stage: &'static str) -> Self {
        Self {
            at: std::time::Instant::now(),
            stage,
        }
    }
    pub fn stage(&mut self, stage: &'static str) {
        self.report(true);
        self.at = std::time::Instant::now();
        self.stage = stage;
    }
    fn report(&self, finished: bool) {
        tracing::debug!(target: "dakia_server::search_timing", stage = self.stage, elapsed_ms = self.at.elapsed().as_secs_f64() * 1000., finished, "search stage");
    }
}
impl Drop for SearchTiming {
    fn drop(&mut self) {
        self.report(false);
    }
}

#[cfg(feature = "test-support")]
type LexicalQuery<'q, DB, Row> =
    sqlx::query::QueryAs<'q, DB, Row, <DB as sqlx::Database>::Arguments<'q>>;

/// Explicit synthetic-fixture diagnostics only. Ordinary binaries do not
/// compile this capture path, and production timing logs never include SQL
/// arguments or query text.
#[cfg(feature = "test-support")]
pub(crate) fn capture_lexical<'q, DB, Row>(
    mut query: LexicalQuery<'q, DB, Row>,
) -> crate::Result<LexicalQuery<'q, DB, Row>>
where
    DB: sqlx::Database,
    DB::Arguments<'q>: sqlx::IntoArguments<'q, DB> + std::fmt::Debug,
    Row: for<'r> sqlx::FromRow<'r, DB::Row> + Send,
{
    use sqlx::Execute;
    let Some(root) = std::env::var_os("DAKIA_TEST_CAPTURE_LEXICAL_SQL") else {
        return Ok(query);
    };
    let sql = query.sql();
    let arguments = query
        .take_arguments()
        .map_err(sqlx::Error::Encode)?
        .ok_or(crate::Error::Invalid)?;
    let root = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join(format!("{}.json", uuid::Uuid::new_v4())),
        serde_json::to_vec(&serde_json::json!({
            "sql": sql,
            "arguments_debug": format!("{arguments:?}"),
            "scope": "explicit test-support synthetic SQL diagnostic"
        }))?,
    )?;
    Ok(sqlx::query_as_with(sql, arguments))
}
