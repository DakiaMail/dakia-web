//! Offline operator tool. Server and tool use the same exclusive lock.
use dakia_storage::{lock_data_directory, Store};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: dakia-admin backup DATA_DIR NEW_BACKUP_DIR KEY_FILE | restore BACKUP_DIR NEW_DATA_DIR KEY_FILE | enable-jobs DATA_DIR KEY_FILE --reconciled".into());
    }
    let key_path = if args[1] == "enable-jobs" {
        &args[3]
    } else {
        &args[4]
    };
    let key = tokio::fs::read(key_path).await?;
    let key: [u8; 32] = key
        .try_into()
        .map_err(|_| "credential key must contain exactly 32 bytes")?;
    match args[1].as_str() {
        "backup" => {
            let _lock = lock_data_directory(&args[2])?;
            let s = Store::open(&args[2], key).await?;
            s.backup(&args[3]).await?;
            println!("Backup completed: {}", args[3]);
        }
        "restore" => {
            Store::restore(&args[2], &args[3], key).await?;
            println!("Restore completed with jobs disabled: {}", args[3]);
        }
        "enable-jobs" => {
            if args[4] != "--reconciled" {
                return Err(
                    "review restored mail and uncertain submissions, then pass --reconciled".into(),
                );
            }
            enable_jobs(&args[2], key).await?;
            println!("Background jobs enabled after operator reconciliation");
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
async fn enable_jobs(data: &str, key: [u8; 32]) -> Result<(), Box<dyn std::error::Error>> {
    let _lock = lock_data_directory(data)?;
    let store = Store::open(data, key).await?;
    let accounts: Vec<(String, String)> = sqlx::query_as("SELECT id,user_id FROM accounts")
        .fetch_all(store.control())
        .await?;
    for (id, user) in accounts {
        let scope = store.authorize(&user, &id).await?;
        store.credential(&scope).await?;
    }
    sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES('jobs_enabled','true')")
        .execute(store.control())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn job_enable_verifies_credentials_before_changing_the_flag() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().to_str().unwrap();
        let store = Store::open(data, [7; 32]).await.unwrap();
        let user = store
            .user("https://test.invalid", "operator")
            .await
            .unwrap();
        let input = dakia_domain::ConnectAccount {
            email: "fixture@example.invalid".into(),
            display_name: "Fixture".into(),
            provider: dakia_domain::ProviderKind::Imap,
            imap_host: "fixture.invalid".into(),
            imap_port: 993,
            imap_tls: dakia_domain::TlsMode::Implicit,
            smtp_host: "fixture.invalid".into(),
            smtp_port: 465,
            smtp_tls: dakia_domain::TlsMode::Implicit,
            username: "fixture".into(),
            password: "synthetic".into(),
            gmail_oauth: None,
        };
        store.connect(&user, &input).await.unwrap();
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES('jobs_enabled','false')")
            .execute(store.control())
            .await
            .unwrap();
        assert!(enable_jobs(data, [8; 32]).await.is_err());
        assert!(!store.jobs_enabled().await.unwrap());
        enable_jobs(data, [7; 32]).await.unwrap();
        assert!(store.jobs_enabled().await.unwrap());
    }
}
