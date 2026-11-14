//! Offline copying of explicitly mapped durable columns into native PostgreSQL.
//! Runtime domain queries never pass through this archive-only conversion.
use crate::*;
use futures_util::TryStreamExt;
use sqlx::{PgConnection, Row, ValueRef};
pub(crate) async fn copy_to_postgres(
    source: &SqlitePool,
    target: &mut PgConnection,
    table: &str,
    mapping: &str,
    account: Option<&str>,
) -> Result<()> {
    let mappings: Vec<_> = mapping
        .split(',')
        .map(|value| {
            let mut parts = value.split(" AS ");
            let destination = parts.next().unwrap();
            let origin = parts.next().unwrap_or(destination);
            (destination, origin)
        })
        .collect();
    let identifier = |name: &str| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    };
    if !identifier(table)
        || mappings
            .iter()
            .any(|(destination, origin)| !identifier(destination) || !identifier(origin))
    {
        return Err(Error::Invalid);
    }
    let types:HashMap<String,String>=sqlx::query_as::<_,(String,String)>("SELECT column_name,data_type FROM information_schema.columns WHERE table_schema=current_schema() AND table_name=$1").bind(table).fetch_all(&mut *target).await?.into_iter().collect();
    let sql = format!(
        "SELECT {} FROM {table}",
        mappings
            .iter()
            .map(|(_, origin)| *origin)
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut rows = sqlx::query(&sql).fetch(source);
    while let Some(row) = rows.try_next().await? {
        let mut insert = sqlx::QueryBuilder::<sqlx::Postgres>::new(format!("INSERT INTO {table}("));
        if account.is_some() {
            insert.push("account_id,");
        }
        insert
            .push(
                mappings
                    .iter()
                    .map(|(dest, _)| *dest)
                    .collect::<Vec<_>>()
                    .join(","),
            )
            .push(") VALUES(");
        if let Some(account) = account {
            insert.push_bind(account).push(",");
        }
        for (n, (dest, _)) in mappings.iter().enumerate() {
            if n > 0 {
                insert.push(",");
            }
            let null = row.try_get_raw(n)?.is_null();
            match types.get(*dest).map(String::as_str) {
                Some("text") => {
                    insert.push_bind(if null {
                        None
                    } else {
                        Some(row.try_get::<String, _>(n)?)
                    });
                }
                Some("bigint") => {
                    insert.push_bind(if null {
                        None
                    } else {
                        Some(row.try_get::<i64, _>(n)?)
                    });
                }
                Some("boolean") => {
                    insert.push_bind(if null {
                        None
                    } else {
                        Some(row.try_get::<bool, _>(n)?)
                    });
                }
                Some("bytea") => {
                    insert.push_bind(if null {
                        None
                    } else {
                        Some(row.try_get::<Vec<u8>, _>(n)?)
                    });
                }
                _ => return Err(Error::Invalid),
            }
        }
        insert.push(")");
        if table == "thread_edges" {
            insert.push(" ON CONFLICT DO NOTHING");
        }
        insert.build().execute(&mut *target).await?;
    }
    Ok(())
}
