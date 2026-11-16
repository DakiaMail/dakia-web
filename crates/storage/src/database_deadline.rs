//! Native limits follow the calling background operation, never migrations.
use crate::Error;
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    Connection, PgConnection, SqliteConnection,
};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Deadline {
    until: Instant,
    active: Arc<AtomicBool>,
    finished: Arc<tokio::sync::Notify>,
    succeeded: Arc<AtomicBool>,
}

struct SqliteDeadline {
    deadline: Deadline,
    handle: usize,
    busy_timeout: i32,
}
impl Drop for SqliteDeadline {
    fn drop(&mut self) {
        // SQLx drops its progress callback under the handle lock, before closing SQLite.
        unsafe {
            libsqlite3_sys::sqlite3_busy_timeout(self.handle as *mut _, self.busy_timeout);
        }
    }
}
unsafe extern "C" fn sqlite_busy(context: *mut std::ffi::c_void, count: i32) -> i32 {
    // The progress callback owns this allocation until it restores the busy timeout.
    let state = unsafe { &*(context as *const SqliteDeadline) };
    if count >= state.busy_timeout
        || !state.deadline.active.load(Ordering::Acquire)
        || Instant::now() >= state.deadline.until
    {
        return 0;
    }
    std::thread::sleep(Duration::from_millis(1));
    1
}
tokio::task_local! { static CURRENT: Deadline; }
struct Cancel(Deadline);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::Release);
        self.0.finished.notify_waiters();
    }
}

/// Bound database execution as well as the caller's wait. Cancellation interrupts SQLite.
pub async fn storage_deadline<T, E: From<Error>>(
    turn: Duration,
    work: impl Future<Output = std::result::Result<T, E>>,
) -> std::result::Result<T, E> {
    let until = Instant::now() + turn;
    let until = CURRENT
        .try_with(|parent| parent.until.min(until))
        .unwrap_or(until);
    let active = Arc::new(AtomicBool::new(true));
    let deadline = Deadline {
        until,
        active,
        finished: Arc::new(tokio::sync::Notify::new()),
        succeeded: Arc::new(AtomicBool::new(false)),
    };
    let _cancel = Cancel(deadline.clone());
    CURRENT
        .scope(deadline.clone(), async {
            let result =
                tokio::time::timeout(until.saturating_duration_since(Instant::now()), work).await;
            match result {
                Err(_) => Err(Error::DeadlineExceeded.into()),
                Ok(Err(_)) if Instant::now() >= until => Err(Error::DeadlineExceeded.into()),
                Ok(result) => {
                    deadline.succeeded.store(result.is_ok(), Ordering::Release);
                    result
                }
            }
        })
        .await
}

pub(crate) async fn sqlite(connection: &mut SqliteConnection) -> sqlx::Result<()> {
    connection.lock_handle().await?.remove_progress_handler();
    if let Ok(deadline) = CURRENT.try_with(Clone::clone) {
        sqlite_until(connection, deadline).await?;
    }
    Ok(())
}

async fn sqlite_until(connection: &mut SqliteConnection, deadline: Deadline) -> sqlx::Result<()> {
    connection.lock_handle().await?.remove_progress_handler();
    let busy_timeout: i32 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(&mut *connection)
        .await?;
    let mut handle = connection.lock_handle().await?;
    let state = Arc::new(SqliteDeadline {
        deadline,
        handle: handle.as_raw_handle().as_ptr() as usize,
        busy_timeout,
    });
    let busy_state = Arc::as_ptr(&state) as *mut std::ffi::c_void;
    handle.set_progress_handler(1000, move || {
        state.deadline.active.load(Ordering::Acquire) && Instant::now() < state.deadline.until
    });
    unsafe {
        libsqlite3_sys::sqlite3_busy_handler(
            handle.as_raw_handle().as_ptr(),
            Some(sqlite_busy),
            busy_state,
        );
    }
    Ok(())
}

pub(crate) async fn sqlite_read_limit(
    connection: &mut SqliteConnection,
    turn: Duration,
    active: Arc<AtomicBool>,
) -> sqlx::Result<()> {
    let until = Instant::now() + turn;
    let until = CURRENT
        .try_with(|parent| parent.until.min(until))
        .unwrap_or(until);
    sqlite_until(
        connection,
        Deadline {
            until,
            active,
            finished: Arc::new(tokio::sync::Notify::new()),
            succeeded: Arc::new(AtomicBool::new(false)),
        },
    )
    .await
}

async fn postgres_limit(
    connection: &mut PgConnection,
    until: Instant,
    local: bool,
) -> sqlx::Result<()> {
    let remaining = until
        .saturating_duration_since(Instant::now())
        .as_millis()
        .clamp(1, i32::MAX as u128);
    // pg_settings exposes milliseconds without interval-string rounding or unit conversion.
    sqlx::query("SELECT set_config(name,(CASE WHEN setting::BIGINT=0 THEN $1 ELSE LEAST(setting::BIGINT,$1) END)::TEXT||'ms',$2) FROM pg_settings WHERE name IN ('statement_timeout','lock_timeout')")
        .bind(remaining as i64).bind(local).execute(connection).await?;
    Ok(())
}

pub(crate) async fn postgres_read_limit(
    connection: &mut PgConnection,
    turn: Duration,
) -> sqlx::Result<()> {
    let until = Instant::now() + turn;
    let until = CURRENT
        .try_with(|parent| parent.until.min(until))
        .unwrap_or(until);
    postgres_limit(connection, until, true).await
}

async fn postgres(connection: &mut PgConnection) -> sqlx::Result<()> {
    sqlx::Executor::execute(
        &mut *connection,
        "RESET statement_timeout; RESET lock_timeout",
    )
    .await?;
    if let Ok(deadline) = CURRENT.try_with(Clone::clone) {
        // Preserve stricter operator limits; RESET on release restores startup settings.
        postgres_limit(connection, deadline.until, false).await?;
    }
    Ok(())
}

// Entries are weak, and each watcher removes its entry when finished, including pool close.
type PostgresLeases = Arc<Mutex<HashMap<i32, Weak<PostgresLease>>>>;
struct PostgresLease {
    released: AtomicBool,
    changed: tokio::sync::Notify,
    done: AtomicBool,
    finished: tokio::sync::Notify,
}
async fn fence(lease: Arc<PostgresLease>) {
    lease.released.store(true, Ordering::Release);
    lease.changed.notify_waiters();
    loop {
        let finished = lease.finished.notified();
        if lease.done.load(Ordering::Acquire) {
            break;
        }
        finished.await;
    }
}
async fn checkout_postgres(
    connection: &mut PgConnection,
    options: PgConnectOptions,
    leases: PostgresLeases,
) -> sqlx::Result<()> {
    postgres(connection).await?;
    let Ok(deadline) = CURRENT.try_with(Clone::clone) else {
        return Ok(());
    };
    let (pid, started): (i32, String) = sqlx::query_as(
        "SELECT pid, backend_start::TEXT FROM pg_stat_activity WHERE pid=pg_backend_pid()",
    )
    .fetch_one(connection)
    .await?;
    let old = leases
        .lock()
        .unwrap()
        .remove(&pid)
        .and_then(|old| old.upgrade());
    if let Some(old) = old {
        fence(old).await;
    }
    let lease = Arc::new(PostgresLease {
        released: AtomicBool::new(false),
        changed: tokio::sync::Notify::new(),
        done: AtomicBool::new(false),
        finished: tokio::sync::Notify::new(),
    });
    leases.lock().unwrap().insert(pid, Arc::downgrade(&lease));
    tokio::spawn(async move {
        // Register notifications before reading flags, so cancellation cannot be lost.
        let stopped = lease.changed.notified();
        let scope_finished = deadline.finished.notified();
        if !lease.released.load(Ordering::Acquire) && deadline.active.load(Ordering::Acquire) {
            tokio::select! {
                _ = stopped => {},
                _ = scope_finished => {},
                _ = tokio::time::sleep(deadline.until.saturating_duration_since(Instant::now())) => {},
            }
        }
        if !lease.released.load(Ordering::Acquire) && !deadline.succeeded.load(Ordering::Acquire) {
            // A separate own-role connection cancels the active statement, even late in a scope.
            // backend_start prevents a closed session's PID reuse from targeting a new session.
            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                let mut cancel = PgConnection::connect_with(&options).await?;
                sqlx::query("SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE pid=$1 AND backend_start::TEXT=$2")
                    .bind(pid).bind(started).execute(&mut cancel).await?;
                cancel.close().await
            }).await;
        }
        let mut entries = leases.lock().unwrap();
        if entries
            .get(&pid)
            .is_some_and(|entry| entry.ptr_eq(&Arc::downgrade(&lease)))
        {
            entries.remove(&pid);
        }
        lease.done.store(true, Ordering::Release);
        lease.finished.notify_waiters();
    });
    Ok(())
}
async fn release_postgres(
    connection: &mut PgConnection,
    leases: PostgresLeases,
) -> sqlx::Result<bool> {
    // Draining an abandoned query is bounded even if the cancellation connection fails.
    let pid: i32 = tokio::time::timeout(
        Duration::from_secs(1),
        sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection),
    )
    .await
    .map_err(|_| sqlx::Error::Protocol("database cleanup deadline exceeded".into()))??;
    let lease = leases
        .lock()
        .unwrap()
        .remove(&pid)
        .and_then(|lease| lease.upgrade());
    // Never lend this backend again until an in-flight cancellation has finished.
    if let Some(lease) = lease {
        fence(lease).await;
    }
    tokio::time::timeout(
        Duration::from_secs(1),
        sqlx::Executor::execute(connection, "RESET statement_timeout; RESET lock_timeout"),
    )
    .await
    .map_err(|_| sqlx::Error::Protocol("database cleanup deadline exceeded".into()))??;
    Ok(true)
}

pub(crate) fn postgres_pool_options(options: &PgConnectOptions) -> PgPoolOptions {
    let leases = PostgresLeases::default();
    let connect_leases = leases.clone();
    let acquire_leases = leases.clone();
    let connect_options = options.clone();
    let acquire_options = options.clone();
    PgPoolOptions::new()
        .after_connect(move |connection, _| {
            Box::pin(checkout_postgres(
                connection,
                connect_options.clone(),
                connect_leases.clone(),
            ))
        })
        .before_acquire(move |connection, _| {
            let work =
                checkout_postgres(connection, acquire_options.clone(), acquire_leases.clone());
            Box::pin(async move {
                work.await?;
                Ok(true)
            })
        })
        .after_release(move |connection, _| Box::pin(release_postgres(connection, leases.clone())))
}

#[cfg(test)]
mod tests;
