//! Offline operator tool. Server and tool use the same exclusive lock.
use dakia_storage::{lock_data_directory, Store};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: dakia-admin backup DATA_DIR NEW_BACKUP_DIR KEY_FILE | restore BACKUP_DIR NEW_DATA_DIR KEY_FILE".into());
    }
    let key = tokio::fs::read(&args[4]).await?;
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
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
