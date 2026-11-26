use crate::Result;
use sqlx::{PgConnection, SqlitePool};

pub(crate) fn normalize(address: &str) -> String {
    address.trim().to_lowercase()
}

// Legacy rows and portable imports only carry the original address.
pub(crate) async fn backfill_sqlite(db: &SqlitePool) -> Result<()> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    loop {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id,json_extract(document,'$.sender.email') FROM messages WHERE sender_key IS NULL ORDER BY id LIMIT 500",
        )
        .fetch_all(&mut *tx)
        .await?;
        if rows.is_empty() {
            break;
        }
        for (id, address) in rows {
            sqlx::query("UPDATE messages SET sender_key=? WHERE id=?")
                .bind(normalize(address.as_deref().unwrap_or_default()))
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn backfill_postgres(tx: &mut PgConnection, account: Option<&str>) -> Result<()> {
    loop {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT account_id,id,sender_address FROM messages WHERE sender_key IS NULL AND ($1::TEXT IS NULL OR account_id=$1) ORDER BY account_id,id COLLATE \"C\" LIMIT 500 FOR UPDATE",
        )
        .bind(account)
        .fetch_all(&mut *tx)
        .await?;
        if rows.is_empty() {
            return Ok(());
        }
        let mut accounts = Vec::with_capacity(rows.len());
        let mut ids = Vec::with_capacity(rows.len());
        let mut keys = Vec::with_capacity(rows.len());
        for (account, id, address) in rows {
            accounts.push(account);
            ids.push(id);
            keys.push(normalize(address.as_deref().unwrap_or_default()));
        }
        sqlx::query(
            "UPDATE messages m SET sender_key=v.key FROM unnest($1::TEXT[],$2::TEXT[],$3::TEXT[]) AS v(account_id,id,key) WHERE m.account_id=v.account_id AND m.id=v.id",
        )
        .bind(&accounts)
        .bind(&ids)
        .bind(&keys)
        .execute(&mut *tx)
        .await?;
    }
}
