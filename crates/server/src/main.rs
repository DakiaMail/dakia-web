use dakia_server::{router_with_shutdown, App, Config};
use dakia_storage::Store;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::env::var("DAKIA_DATA_DIR")?;
    let _lock = dakia_storage::lock_data_directory(&data)?;
    let retained = dakia_server::diagnostics::Retained::open(std::path::Path::new(&data))?;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::EnvFilter::new(
                "dakia_server=info,tower_http=info",
            )),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(retained)
                .with_filter(tracing_subscriber::filter::filter_fn(|m| {
                    dakia_server::diagnostics::retained_target(m.target())
                })),
        )
        .init();
    tracing::info!(target: "dakia_server::diagnostics", schema_version = 2,
        max_files = 32, max_file_bytes = 8388608, "mail diagnostic retention active");
    // A quiet service still provides continuity evidence for window exports.
    let diagnostic_heartbeat = tokio::spawn(async {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            tracing::info!(target: "dakia_server::diagnostics", schema_version = 2, "mail_heartbeat");
        }
    });
    let required =
        |name: &str| std::env::var(name).map_err(|_| format!("required configuration: {name}"));
    let config = Config {
        origin: required("DAKIA_ORIGIN")?,
        issuer: std::env::var("DAKIA_OIDC_ISSUER").unwrap_or_default(),
        client_id: std::env::var("DAKIA_OIDC_CLIENT_ID").unwrap_or_default(),
        client_secret: match std::env::var("DAKIA_OIDC_SECRET_FILE") {
            Ok(path) => tokio::fs::read_to_string(path).await?.trim().into(),
            Err(_) => String::new(),
        },
        web_root: std::env::var("DAKIA_WEB_ROOT").unwrap_or_else(|_| "apps/web/dist".into()),
    };
    config.validate()?;
    let key_path = required("DAKIA_CREDENTIAL_KEY_FILE")?;
    dakia_server::initialize_credential_key(&data, &key_path)?;
    let key = tokio::fs::read(&key_path).await?;
    let key: [u8; 32] = key
        .try_into()
        .map_err(|_| "credential key must contain exactly 32 bytes")?;

    let attachment_budget = std::env::var("DAKIA_DURABLE_ATTACHMENT_BUDGET_BYTES")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(1024 * 1024 * 1024);
    let store = Store::open_with_attachment_budget(data, key, attachment_budget).await?;
    if config.issuer.is_empty() {
        dakia_server::initialize_local_login(&store, &key_path).await?;
    }
    let state = App {
        store,
        config: Arc::new(config),
        mail: Default::default(),
    };
    let executor = tokio::spawn(dakia_server::executor::run(state.clone()));
    let models = tokio::spawn(dakia_server::models::run(state.clone()));
    let push = tokio::spawn(dakia_server::push::run(state.clone()));
    let idle = tokio::spawn(dakia_server::idle::run(state.clone()));
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let app = router_with_shutdown(state, shutdown.clone());
    let addr = std::env::var("DAKIA_LISTEN").unwrap_or_else(|_| "127.0.0.1:3025".into());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("install SIGTERM handler");
            tokio::select! {_ = tokio::signal::ctrl_c()=>{}, _ = terminate.recv()=>{}}
            shutdown.send_replace(true);
        })
        .await?;
    diagnostic_heartbeat.abort();
    executor.abort();
    models.abort();
    push.abort();
    idle.abort();
    Ok(())
}
