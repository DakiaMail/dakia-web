use super::*;
impl Store {
    /// A failed read contradicts a completed snapshot. Revoke its cheap-check
    /// proof and schedule membership reconciliation, never delete mail on a
    /// fetch error. Compare the reference to avoid invalidating a newer view.
    pub async fn invalidate_stale_reference(
        &self,
        a: &AuthorizedAccount,
        message_id: &str,
        folder: &str,
        reference: &str,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let changed = sqlx::query("UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=0 WHERE folder=? AND completed_at IS NOT NULL AND EXISTS(SELECT 1 FROM provider_documents WHERE key=? AND document=?) AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=? AND EXISTS(SELECT 1 FROM json_each(document,'$.folder_ids') WHERE value=?))")
            .bind(folder).bind(format!("reference:{message_id}")).bind(reference)
            .bind(message_id).bind(reference).bind(folder).execute(&mut *tx).await?.rows_affected() == 1;
        if changed {
            sqlx::query("DELETE FROM provider_documents WHERE key IN (?,?)")
                .bind(format!("sync:modseq:{folder}"))
                .bind(format!("sync:verified:{folder}"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }
    pub async fn scan_generation(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<Option<i64>> {
        let db = self.db(a).await?;
        Ok(
            sqlx::query_scalar("SELECT generation FROM folder_scans WHERE folder=?")
                .bind(folder)
                .fetch_optional(&db)
                .await?,
        )
    }
    /// Read terminal traversals without rolling their generations forward.
    /// Limited exposure can finish traversal without certifying coverage.
    pub async fn terminal_scan_folders(&self, a: &AuthorizedAccount) -> Result<Vec<String>> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        Ok(sqlx::query_scalar("SELECT folder FROM folder_scans WHERE continuation IS NULL AND (completed_at IS NOT NULL OR coverage_limited=1)")
            .fetch_all(&db).await?)
    }
    /// Only a completed, fully exposed snapshot may be refreshed without a scan.
    pub async fn completed_scan_due(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<Option<i64>> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let generation: Option<i64> = sqlx::query_scalar("SELECT generation FROM folder_scans WHERE folder=? AND continuation IS NULL AND completed_at IS NOT NULL AND coverage_limited=0 AND COALESCE(last_scanned_at,0)<=?")
            .bind(folder).bind(Utc::now().timestamp()-300).fetch_optional(&mut *tx).await?;
        let seeded = if generation.is_some() {
            self.seed_projected_memberships(&mut tx, folder).await?
        } else {
            0
        };
        if seeded > 0 {
            // Metadata can predate membership tracking or arrive through a
            // filtered search. Its projection is not covered by the old proof.
            sqlx::query("UPDATE folder_scans SET generation=generation+1,completed_at=NULL,continuation=NULL,coverage_limited=0 WHERE folder=?")
                .bind(folder).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM provider_documents WHERE key IN (?,?)")
                .bind(format!("sync:modseq:{folder}"))
                .bind(format!("sync:verified:{folder}"))
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            tracing::info!(target: "dakia_server::diagnostics", operation = "snapshot_membership_repair", rows = seeded, outcome = "scheduled", "mail_work");
            return Ok(None);
        }
        tx.commit().await?;
        Ok(generation)
    }
    /// Guard against resets during provider verification. Keep coverage and
    /// memberships intact; this is not an empty page or a new generation.
    pub async fn confirm_unchanged_scan(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
        generation: i64,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let changed = sqlx::query("UPDATE folder_scans SET last_scanned_at=? WHERE folder=? AND generation=? AND continuation IS NULL AND completed_at IS NOT NULL AND coverage_limited=0")
            .bind(Utc::now().timestamp()).bind(folder).bind(generation).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO provider_documents(key,document) SELECT ?,json_object('generation',generation,'uidvalidity',uidvalidity,'verified_at',last_scanned_at) FROM folder_scans WHERE folder=? ON CONFLICT(key) DO UPDATE SET document=excluded.document")
            .bind(format!("sync:verified:{folder}")).bind(folder).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    /// A recent positive unchanged check covers all flags only for a fully
    /// completed snapshot. Resets, epoch changes and limited exposure revoke it.
    pub async fn verified_snapshot_current(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<bool> {
        let db = self.db(a).await?;
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM folder_scans f JOIN provider_documents v ON v.key=? JOIN provider_documents e ON e.key=? WHERE f.folder=? AND f.completed_at IS NOT NULL AND f.continuation IS NULL AND f.coverage_limited=0 AND f.generation=json_extract(v.document,'$.generation') AND f.uidvalidity=json_extract(v.document,'$.uidvalidity') AND f.uidvalidity=CAST(e.document AS INTEGER) AND json_extract(v.document,'$.verified_at')>? AND json_extract(v.document,'$.verified_at')<=?)")
            .bind(format!("sync:verified:{folder}")).bind(format!("thread:epoch:{folder}")).bind(folder)
            .bind(Utc::now().timestamp()-300).bind(Utc::now().timestamp()).fetch_one(&db).await?)
    }
    pub async fn scan_state(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<Option<(i64, Option<String>)>> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO folder_scans(folder) VALUES(?)")
            .bind(folder)
            .execute(&mut *tx)
            .await?;
        // Terminal limited scans cool down too, without certifying coverage.
        // The conditional update also prevents concurrent readers from rolling
        // the same finished generation forward more than once.
        sqlx::query("UPDATE folder_scans SET generation=generation+1,completed_at=NULL,coverage_limited=0 WHERE folder=? AND continuation IS NULL AND (completed_at IS NOT NULL OR coverage_limited=1) AND COALESCE(last_scanned_at,0)<=?").bind(folder).bind(Utc::now().timestamp()-300).execute(&mut *tx).await?;
        let row = sqlx::query(
            "SELECT generation,continuation,completed_at,coverage_limited FROM folder_scans WHERE folder=?",
        )
        .bind(folder)
        .fetch_one(&mut *tx)
        .await?;
        if row.get::<Option<i64>, _>("completed_at").is_some()
            || (row.get::<bool, _>("coverage_limited")
                && row.get::<Option<String>, _>("continuation").is_none())
        {
            tx.commit().await?;
            return Ok(None);
        }
        if row.get::<Option<String>, _>("continuation").is_none() {
            // Capture untracked projections before the provider chooses its
            // traversal boundary. Never add late foreground discoveries to an
            // older traversal's deletion set at completion.
            self.seed_projected_memberships(&mut tx, folder).await?;
        }
        tx.commit().await?;
        Ok(Some((row.get("generation"), row.get("continuation"))))
    }
    pub async fn reset_scan(&self, a: &AuthorizedAccount, folder: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        sqlx::query("UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=0 WHERE folder=?").bind(folder).execute(&db).await?;
        Ok(())
    }
    /// Commit cursor and observed memberships together. Prune only after a complete successful scan.
    pub async fn record_scan(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
        generation: i64,
        page: &SearchPage,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        // A partially exposed provider folder must never certify full coverage
        // or prune memberships it could not observe.
        let previous_limited: bool =
            sqlx::query_scalar("SELECT coverage_limited FROM folder_scans WHERE folder=?")
                .bind(folder)
                .fetch_one(&mut *tx)
                .await?;
        let limited = previous_limited
            || page.coverage.is_empty()
            || page
                .coverage
                .iter()
                .any(|c| c.state != CoverageState::Complete);
        let complete = page.continuation.is_none() && !limited;
        let changed = sqlx::query(
            "UPDATE folder_scans SET continuation=?,completed_at=?,last_scanned_at=COALESCE(?,last_scanned_at),coverage_limited=?,uidvalidity=(SELECT CAST(document AS INTEGER) FROM provider_documents WHERE key='thread:epoch:'||folder_scans.folder) WHERE folder=? AND generation=? AND completed_at IS NULL AND NOT (coverage_limited=1 AND continuation IS NULL)",
        )
        .bind(&page.continuation)
        .bind(complete.then(|| Utc::now().timestamp()))
        .bind(page.continuation.is_none().then(|| Utc::now().timestamp()))
        .bind(limited)
        .bind(folder)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        self.observe_memberships(&mut tx, a, folder, generation, &page.rows)
            .await?;
        if complete {
            self.finish_snapshot(&mut tx, folder, generation).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Arrival observations plus watermark, never snapshot completion/pruning.
    /// Replays after a crash between ingestion and this transaction are safe.
    pub async fn record_incremental(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
        rows: &[Message],
        checkpoint: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO folder_scans(folder) VALUES(?)")
            .bind(folder)
            .execute(&mut *tx)
            .await?;
        let generation: i64 =
            sqlx::query_scalar("SELECT generation FROM folder_scans WHERE folder=?")
                .bind(folder)
                .fetch_one(&mut *tx)
                .await?;
        self.observe_memberships(&mut tx, a, folder, generation, rows)
            .await?;
        // Establish a quiet bootstrap even during unfinished history. Set this
        // AFTER observations so future-dated initial rows cannot emit alerts.
        sqlx::query(
            "UPDATE folder_scans SET initial_boundary=COALESCE(initial_boundary,?) WHERE folder=?",
        )
        .bind(Utc::now().timestamp())
        .bind(folder)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document")
            .bind(format!("sync:head:{folder}")).bind(checkpoint).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn observe_memberships(
        &self,
        tx: &mut sqlx::SqliteConnection,
        a: &AuthorizedAccount,
        folder: &str,
        generation: i64,
        rows: &[Message],
    ) -> Result<()> {
        let boundary: Option<i64> =
            sqlx::query_scalar("SELECT initial_boundary FROM folder_scans WHERE folder=?")
                .bind(folder)
                .fetch_one(&mut *tx)
                .await?;
        let subscriptions = self.subscriptions(a).await?;
        let rules = self.rules(&a.user_id).await?;
        for message in rows {
            if message.account_id != a.id {
                return Err(Error::Invalid);
            }
            let known: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM folder_memberships WHERE folder=? AND message_id=?)",
            )
            .bind(folder)
            .bind(&message.id)
            .fetch_one(&mut *tx)
            .await?;
            if !known
                && folder.eq_ignore_ascii_case("INBOX")
                && !rules.iter().any(|rule| {
                    rule.enabled && {
                        let RuleCondition::SenderIs { email } = &rule.condition;
                        message.sender.email.trim().to_lowercase() == *email
                    }
                })
                && boundary.is_some_and(|b| {
                    DateTime::parse_from_rfc3339(&message.received_at)
                        .is_ok_and(|d| d.timestamp() > b)
                })
            {
                let inserted =
                    sqlx::query("INSERT OR IGNORE INTO notification_dedup(message_id) VALUES(?)")
                        .bind(&message.id)
                        .execute(&mut *tx)
                        .await?;
                if inserted.rows_affected() == 1 {
                    sqlx::query("DELETE FROM snoozes WHERE thread_id=? AND arrival_boundary<?")
                        .bind(&message.thread_id)
                        .bind(&message.received_at)
                        .execute(&mut *tx)
                        .await?;
                    for subscription in &subscriptions {
                        let id = subscription["id"].as_str().ok_or(Error::Invalid)?;
                        let job = serde_json::json!({"id":format!("{}:{}:{id}",a.id,message.id),"account_id":a.id,"user_id":a.user_id,"message_id":message.id,"subscription_id":id,"scope":{"account_id":a.id,"user_id":a.user_id},"subscription":subscription,"deadline":(Utc::now()+Duration::minutes(5)).to_rfc3339(),"initial_sync":false});
                        sqlx::query("INSERT OR IGNORE INTO push_jobs(message_id,subscription_id,document,state) VALUES(?,?,?,CASE WHEN EXISTS(SELECT 1 FROM messages WHERE id=? AND COALESCE(category_override,json_extract(document,'$.category'),'')='newsletters') THEN 'skipped' ELSE 'pending' END)").bind(&message.id).bind(id).bind(serde_json::to_string(&job)?).bind(&message.id).execute(&mut *tx).await?;
                    }
                }
            }
            sqlx::query("INSERT INTO folder_memberships(folder,message_id,generation) VALUES(?,?,?) ON CONFLICT(folder,message_id) DO UPDATE SET generation=excluded.generation")
                .bind(folder).bind(&message.id).bind(generation).execute(&mut *tx).await?;
        }
        Ok(())
    }
    pub(crate) async fn finish_snapshot(
        &self,
        tx: &mut sqlx::SqliteConnection,
        folder: &str,
        generation: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE folder_scans SET initial_boundary=COALESCE(initial_boundary,?) WHERE folder=?",
        )
        .bind(Utc::now().timestamp())
        .bind(folder)
        .execute(&mut *tx)
        .await?;
        // A filtered pending archive is not evidence of provider deletion.
        // Preserve canonical memberships so a failed archive can reappear.
        let stale: Vec<String> = sqlx::query_scalar("SELECT messages.document FROM messages JOIN folder_memberships ON messages.id=folder_memberships.message_id WHERE folder_memberships.folder=? AND folder_memberships.generation<>? AND NOT EXISTS(SELECT 1 FROM operations o WHERE json_extract(o.document,'$.target_id')=messages.id AND json_extract(o.document,'$.status')='pending' AND json_extract(o.document,'$.change.kind') IN ('archive','spam'))").bind(folder).bind(generation).fetch_all(&mut *tx).await?;
        for raw in stale {
            let mut message: Message = serde_json::from_str(&raw)?;
            message.folder_ids.retain(|f| {
                f != folder && !(folder.eq_ignore_ascii_case("INBOX") && f == "\\Inbox")
            });
            message.revision = message.revision.checked_add(1).ok_or(Error::Invalid)?;
            sqlx::query("UPDATE messages SET revision=?,document=? WHERE id=?")
                .bind(message.revision)
                .bind(serde_json::to_string(&message)?)
                .bind(&message.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO events(revision,kind,target_id) VALUES(?,'message_changed',?)",
            )
            .bind(message.revision)
            .bind(&message.id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("DELETE FROM folder_memberships WHERE folder=? AND generation<>? AND NOT EXISTS(SELECT 1 FROM operations o WHERE json_extract(o.document,'$.target_id')=folder_memberships.message_id AND json_extract(o.document,'$.status')='pending' AND json_extract(o.document,'$.change.kind') IN ('archive','spam'))")
                .bind(folder)
                .bind(generation)
                .execute(&mut *tx)
                .await?;
        Ok(())
    }

    pub(crate) async fn seed_projected_memberships(
        &self,
        tx: &mut sqlx::SqliteConnection,
        folder: &str,
    ) -> Result<u64> {
        Ok(sqlx::query("INSERT OR IGNORE INTO folder_memberships(folder,message_id,generation) SELECT ?,m.id,-1 FROM messages m WHERE EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') WHERE value=?)")
            .bind(folder).bind(folder).execute(tx).await?.rows_affected())
    }
}
