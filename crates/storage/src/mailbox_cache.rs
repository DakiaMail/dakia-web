use super::*;
impl MailboxTransaction<'_> {
    async fn learn_recipients(
        &mut self,
        source: &str,
        addresses: &[Address],
        at: &str,
    ) -> Result<()> {
        if source.is_empty() || addresses.len() > 1000 {
            return Err(Error::Invalid);
        }
        let at = DateTime::parse_from_rfc3339(at)
            .map_err(|_| Error::Invalid)?
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        for address in addresses {
            if address.email.len() > 320
                || !address.email.contains('@')
                || address.email.chars().any(char::is_control)
            {
                continue;
            }
            let email = address.email.trim();
            let raw = serde_json::to_string(address)?;
            write!(self,"INSERT INTO recipient_evidence(source_id,email,document,observed_at) VALUES(?,?,?,?) ON CONFLICT(source_id,email) DO UPDATE SET document=excluded.document,observed_at=excluded.observed_at",[source,email,&raw,&at],"INSERT INTO recipient_evidence(account_id,source_id,email,document,observed_at) VALUES($1,$2,$3,$4,$5) ON CONFLICT(account_id,source_id,email_key) DO UPDATE SET document=excluded.document,observed_at=excluded.observed_at",[&self.account,source,email,&raw,&at]);
        }
        Ok(())
    }
    async fn message_ids(&mut self, providers: &[String]) -> Result<Vec<String>> {
        let mut ids = Vec::with_capacity(providers.len());
        for provider in providers {
            let candidate = Uuid::new_v4().to_string();
            write!(self,"INSERT INTO message_identities(provider_id,message_id) VALUES(?,?) ON CONFLICT DO NOTHING",[provider,&candidate],"INSERT INTO message_identities(account_id,provider_id,message_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",[&self.account,provider,&candidate]);
            ids.push(value!(
                self,
                "SELECT message_id FROM message_identities WHERE provider_id=?",
                [provider],
                "SELECT message_id FROM message_identities WHERE account_id=$1 AND provider_id=$2",
                [&self.account, provider],
                fetch_one
            ));
        }
        Ok(ids)
    }
    async fn put_cache(
        &mut self,
        message: &str,
        blob: &str,
        size: u64,
        budget: u64,
    ) -> Result<Vec<String>> {
        if size > budget {
            return Err(Error::Invalid);
        }
        let size = i64::try_from(size).map_err(|_| Error::Invalid)?;
        let at = now();
        write!(self,"UPDATE received_cache SET last_access=last_access WHERE message_id=?",[message],"UPDATE received_cache SET last_access=last_access WHERE account_id=$1 AND message_id=$2",[&self.account,message]);
        let mut evicted: Vec<String> = value!(
            self,
            "SELECT blob_id FROM received_cache WHERE message_id=?",
            [message],
            "SELECT blob_id FROM received_cache WHERE account_id=$1 AND message_id=$2",
            [&self.account, message],
            fetch_all
        );
        write!(self,"INSERT INTO received_cache(message_id,blob_id,size_bytes,last_access) VALUES(?,?,?,?) ON CONFLICT(message_id) DO UPDATE SET blob_id=excluded.blob_id,size_bytes=excluded.size_bytes,last_access=excluded.last_access",[message,blob,size,&at],"INSERT INTO received_cache(account_id,message_id,blob_id,size_bytes,last_access) VALUES($1,$2,$3,$4,$5) ON CONFLICT(account_id,message_id) DO UPDATE SET blob_id=excluded.blob_id,size_bytes=excluded.size_bytes,last_access=excluded.last_access",[&self.account,message,blob,size,&at]);
        let mut total: i64 = value!(
            self,
            "SELECT COALESCE(SUM(size_bytes),0) FROM received_cache",
            [],
            "SELECT COALESCE(SUM(size_bytes),0)::BIGINT FROM received_cache WHERE account_id=$1",
            [&self.account],
            fetch_one
        );
        let rows:Vec<(String,String,i64)>=tuples!(self,"SELECT c.message_id,c.blob_id,c.size_bytes FROM received_cache c JOIN messages m ON m.id=c.message_id ORDER BY json_extract(m.document,'$.starred'),c.last_access,c.message_id",[],"SELECT c.message_id,c.blob_id,c.size_bytes FROM received_cache c JOIN messages m ON m.account_id=c.account_id AND m.id=c.message_id WHERE c.account_id=$1 ORDER BY m.starred,c.last_access,c.message_id COLLATE \"C\"",[&self.account],fetch_all);
        for (message, blob, size) in rows {
            if total as u64 <= budget {
                break;
            }
            total -= size;
            evicted.push(blob);
            write!(
                self,
                "DELETE FROM received_cache WHERE message_id=?",
                [&message],
                "DELETE FROM received_cache WHERE account_id=$1 AND message_id=$2",
                [&self.account, &message]
            );
        }
        Ok(evicted)
    }
}
impl AccountRepository {
    pub(crate) async fn learn_recipients(
        &self,
        source: &str,
        addresses: &[Address],
        at: &str,
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope.learn_recipients(source, addresses, at).await
        })
    }
    pub(crate) async fn recipient_suggestions(&self, needle: &str) -> Result<Vec<Address>> {
        // Preserve SQLite's ASCII case folding, including NOCASE identities.
        // Pick the newest matching evidence explicitly instead of relying on a
        // backend-specific choice of a bare document in a grouped query.
        let needle = needle.to_ascii_lowercase();
        transaction!(self, tx, local, false, {
            let rows:Vec<String>=scalar!(local,tx,"WITH matches AS (SELECT document,lower(email) AS email_key,observed_at,ROW_NUMBER() OVER (PARTITION BY email ORDER BY observed_at DESC,source_id) AS chosen,COUNT(*) OVER (PARTITION BY email) AS score FROM recipient_evidence WHERE instr(lower(email),?)>0 OR instr(lower(COALESCE(json_extract(document,'$.name'),'')),?)>0) SELECT document FROM matches WHERE chosen=1 ORDER BY score DESC,observed_at DESC,email_key LIMIT 20",[&needle,&needle],"WITH matches AS (SELECT document,email_key,observed_at,ROW_NUMBER() OVER (PARTITION BY email_key ORDER BY observed_at DESC,source_id COLLATE \"C\") AS chosen,COUNT(*) OVER (PARTITION BY email_key) AS score FROM recipient_evidence WHERE account_id=$1 AND (position($2 IN email_key)>0 OR position($2 IN translate(COALESCE(document::jsonb->>'name',''),'ABCDEFGHIJKLMNOPQRSTUVWXYZ','abcdefghijklmnopqrstuvwxyz'))>0)) SELECT document FROM matches WHERE chosen=1 ORDER BY score DESC,observed_at DESC,email_key COLLATE \"C\" LIMIT 20",[&self.account,&needle],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn known_recipient(&self, email: &str) -> Result<bool> {
        let email = email.trim().to_ascii_lowercase();
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM recipient_evidence WHERE email=?)",[&email],"SELECT EXISTS(SELECT 1 FROM recipient_evidence WHERE account_id=$1 AND email_key=$2)",[&self.account,&email],fetch_one))
        })
    }
    pub(crate) async fn message_ids(&self, providers: &[String]) -> Result<Vec<String>> {
        scoped!(self, scope, true, { scope.message_ids(providers).await })
    }
    pub(crate) async fn known_provider_message(&self, provider: &str) -> Result<Option<Message>> {
        transaction!(self, tx, local, false, {
            let raw:Option<String>=scalar!(local,tx,"SELECT m.document FROM message_identities i JOIN messages m ON m.id=i.message_id WHERE i.provider_id=?",[provider],"SELECT m.document FROM message_identities i JOIN messages m ON m.account_id=i.account_id AND m.id=i.message_id WHERE i.account_id=$1 AND i.provider_id=$2",[&self.account,provider],fetch_optional);
            raw.map(|raw| Ok(serde_json::from_str(&raw)?)).transpose()
        })
    }
    pub(crate) async fn starred_rows(
        &self,
        before: Option<(&str, &str)>,
        category: Option<&str>,
        filter: &ClassificationFilter,
    ) -> Result<Vec<Message>> {
        let filter = serde_json::to_string(filter)?;
        let date = before.map(|b| b.0);
        let id = before.map(|b| b.1);
        transaction!(self, tx, local, false, {
            let rows:Vec<String>=scalar!(local,tx,&format!("SELECT document FROM messages m CROSS JOIN (SELECT ? AS filter) cf WHERE json_extract(document,'$.starred')=1 AND json_array_length(document,'$.folder_ids')>0 AND CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END AND (? IS NULL OR json_extract(document,'$.category') IS NULL OR json_extract(document,'$.category')=?) AND (? IS NULL OR (json_extract(document,'$.received_at'),id)<(?,?)) {classification} ORDER BY json_extract(document,'$.received_at') DESC,id DESC LIMIT 51",classification=crate::classification::CLASSIFICATION_SQL),[&filter,category,category,date,date,id],&format!("SELECT m.document FROM messages m CROSS JOIN (SELECT $5::TEXT AS filter) cf WHERE account_id=$1 AND starred AND cardinality(folder_ids)>0 AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=$1 AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch) AND ($2::TEXT IS NULL OR category IS NULL OR category=$2) AND ($3::TEXT IS NULL OR (received_at,id)<($3,$4)) {classification} ORDER BY received_at DESC,id DESC LIMIT 51",classification=crate::classification::CLASSIFICATION_PG_SQL),[&self.account,category,date,id,&filter],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn starred_cache_rows(&self, after: &str) -> Result<Vec<Message>> {
        transaction!(self, tx, local, false, {
            let rows:Vec<String>=scalar!(local,tx,"SELECT document FROM messages m WHERE id>? AND json_extract(document,'$.starred')=1 AND json_array_length(document,'$.folder_ids')>0 AND CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END ORDER BY id LIMIT 2",[after],"SELECT m.document FROM messages m WHERE account_id=$1 AND id>$2 AND starred AND cardinality(folder_ids)>0 AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=$1 AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch) ORDER BY id LIMIT 2",[&self.account,after],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn put_cache(
        &self,
        message: &str,
        blob: &str,
        size: u64,
        budget: u64,
    ) -> Result<Vec<String>> {
        scoped!(self, scope, true, {
            scope.put_cache(message, blob, size, budget).await
        })
    }
    pub(crate) async fn cache_blob(&self, message: &str) -> Result<Option<String>> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(
                local,
                tx,
                "SELECT blob_id FROM received_cache WHERE message_id=?",
                [message],
                "SELECT blob_id FROM received_cache WHERE account_id=$1 AND message_id=$2",
                [&self.account, message],
                fetch_optional
            ))
        })
    }
    pub(crate) async fn cache_blobs(&self) -> Result<Vec<String>> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(
                local,
                tx,
                "SELECT blob_id FROM received_cache",
                [],
                "SELECT blob_id FROM received_cache WHERE account_id=$1",
                [&self.account],
                fetch_all
            ))
        })
    }
    pub(crate) async fn touch_cache(&self, message: &str) -> Result<()> {
        let at = now();
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE received_cache SET last_access=? WHERE message_id=?",
                [&at, message],
                "UPDATE received_cache SET last_access=$1 WHERE account_id=$2 AND message_id=$3",
                [&at, &self.account, message]
            );
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn evict_received(&self) -> Result<()> {
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "DELETE FROM received_cache",
                [],
                "DELETE FROM received_cache WHERE account_id=$1",
                [&self.account]
            );
            tx.commit().await?;
            Ok(())
        })
    }
}
