//! Operator-only release discovery. Requests never include installation or user data.
use super::*;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const API: &str = "https://api.github.com/repos/DakiaMail/dakia-web/releases";
const IMAGE: &str = "ghcr.io/dakiamail/dakia-web@sha256:";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub version: String,
    pub revision: String,
    pub image: String,
    pub notes_url: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub installed_version: Option<String>,
    pub status: &'static str,
    pub checked_at: Option<i64>,
    pub available: Option<Release>,
}
#[derive(Deserialize)]
struct Published {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Manifest {
    format: u32,
    version: String,
    revision: String,
    image: String,
}
struct Cache {
    until: Option<Instant>,
    value: Status,
}
pub struct ReleaseChecker {
    enabled: bool,
    installed: Option<semver::Version>,
    cache: Mutex<Cache>,
}
fn version(tag: &str) -> Option<semver::Version> {
    tag.strip_prefix('v')
        .filter(|s| !s.contains('+'))
        .and_then(|s| semver::Version::parse(s).ok())
}
fn select(releases: Vec<Published>) -> Option<Published> {
    let mut candidates: Vec<_> = releases
        .into_iter()
        .filter_map(|r| {
            let v = version(&r.tag_name)?;
            (!r.draft && r.prerelease == !v.pre.is_empty()).then_some((v, r))
        })
        .collect();
    if candidates.iter().any(|(v, _)| v.pre.is_empty()) {
        candidates.retain(|(v, _)| v.pre.is_empty());
    }
    candidates
        .into_iter()
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r)
}
fn validate(m: Manifest, tag: &str) -> std::result::Result<Release, ()> {
    let digest = m.image.strip_prefix(IMAGE).ok_or(())?;
    if m.format != 1
        || m.version != tag
        || version(tag).is_none()
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || m.revision.len() != 40
        || !m
            .revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(());
    }
    Ok(Release {
        version: m.version,
        revision: m.revision,
        image: m.image,
        notes_url: format!("https://github.com/DakiaMail/dakia-web/releases/tag/{tag}"),
    })
}
async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    limit: usize,
) -> std::result::Result<T, ()> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > limit {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}
impl ReleaseChecker {
    pub fn from_environment() -> Self {
        Self::new(
            std::env::var("DAKIA_RELEASE_CHECKS").as_deref() != Ok("false"),
            option_env!("DAKIA_RELEASE_VERSION").and_then(version),
        )
    }
    fn new(enabled: bool, installed: Option<semver::Version>) -> Self {
        Self {
            enabled,
            installed: installed.clone(),
            cache: Mutex::new(Cache {
                until: None,
                value: Status {
                    installed_version: installed.map(|v| format!("v{v}")),
                    status: "unknown",
                    checked_at: None,
                    available: None,
                },
            }),
        }
    }
    async fn fetch(&self) -> std::result::Result<Option<Release>, ()> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Dakia release checker")
            .build()
            .map_err(|_| ())?;
        // Paginate: preview-heavy repositories must not hide older stable releases.
        let mut releases = Vec::new();
        for page in 1..=10 {
            let response = client
                .get(API)
                .query(&[("per_page", 100), ("page", page)])
                .send()
                .await
                .map_err(|_| ())?
                .error_for_status()
                .map_err(|_| ())?;
            let batch: Vec<Published> = bounded_json(response, 4 * 1024 * 1024).await?;
            let done = batch.len() < 100;
            releases.extend(batch);
            if done {
                break;
            }
            if page == 10 {
                return Err(());
            }
        }
        let Some(release) = select(releases) else {
            return Ok(None);
        };
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == "dakia-release.json")
            .ok_or(())?;
        let prefix = format!(
            "https://github.com/DakiaMail/dakia-web/releases/download/{}/",
            release.tag_name
        );
        if asset.browser_download_url != format!("{prefix}dakia-release.json") {
            return Err(());
        }
        let response = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .map_err(|_| ())?
            .error_for_status()
            .map_err(|_| ())?;
        let manifest: Manifest = bounded_json(response, 16384).await?;
        let release = validate(manifest, &release.tag_name)?;
        Ok((version(&release.version).as_ref() > self.installed.as_ref()).then_some(release))
    }
    async fn check(&self) -> Status {
        self.check_with(|| self.fetch()).await
    }
    async fn check_with<F, Fut>(&self, fetch: F) -> Status
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Option<Release>, ()>>,
    {
        // Holding one async lock through discovery coalesces concurrent owner tabs.
        let mut cache = self.cache.lock().await;
        if !self.enabled || self.installed.is_none() {
            cache.value.status = if self.enabled { "unknown" } else { "disabled" };
            return cache.value.clone();
        }
        if cache.until.is_some_and(|until| Instant::now() < until) {
            return cache.value.clone();
        }
        match fetch().await {
            Ok(available) => {
                cache.value.available = available;
                cache.value.status = "checked";
                cache.value.checked_at = Some(Utc::now().timestamp());
                cache.until = Some(Instant::now() + Duration::from_secs(86400));
            }
            Err(()) => {
                cache.value.status = "unavailable";
                cache.until = Some(Instant::now() + Duration::from_secs(3600));
            }
        }
        cache.value.clone()
    }
}
pub(super) async fn status(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Extension(checker): Extension<Arc<ReleaseChecker>>,
) -> Result<Json<Status>> {
    local_users::require_owner(&app, &id).await?;
    Ok(Json(checker.check().await))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str, draft: bool, preview: bool) -> Published {
        Published {
            tag_name: tag.into(),
            draft,
            prerelease: preview,
            assets: vec![],
        }
    }
    #[test]
    fn selection_and_semver() {
        assert_eq!(
            select(vec![
                release("v1.9.0", false, false),
                release("v1.10.0", false, false),
                release("v2.0.0-beta.1", false, true),
                release("v3.0.0", true, false)
            ])
            .unwrap()
            .tag_name,
            "v1.10.0"
        );
        assert_eq!(
            select(vec![
                release("v0.2.0-beta.2", false, true),
                release("v0.2.0-beta.10", false, true)
            ])
            .unwrap()
            .tag_name,
            "v0.2.0-beta.10"
        );
        assert!(select(vec![
            release("sha-123", false, false),
            release("v1.0.0-beta", false, false)
        ])
        .is_none());
    }
    #[test]
    fn manifest_validation() {
        let m = || Manifest {
            format: 1,
            version: "v1.0.0".into(),
            revision: "a".repeat(40),
            image: format!("{IMAGE}{}", "b".repeat(64)),
        };
        assert!(validate(m(), "v1.0.0").is_ok());
        assert!(validate(m(), "v2.0.0").is_err());
        let mut invalid = m();
        invalid.format = 2;
        assert!(validate(invalid, "v1.0.0").is_err());
    }
    #[tokio::test]
    async fn unknown_and_disabled_do_not_contact_github() {
        assert_eq!(
            ReleaseChecker::new(true, None).check().await.status,
            "unknown"
        );
        assert_eq!(
            ReleaseChecker::new(false, version("v1.0.0"))
                .check()
                .await
                .status,
            "disabled"
        );
    }
    #[tokio::test]
    async fn caches_success_failure_and_coalesces_requests() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checker = ReleaseChecker::new(true, version("v1.0.0"));
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(None)
        };
        let (a, b) = tokio::join!(checker.check_with(fetch), checker.check_with(fetch));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(a.status, "checked");
        assert_eq!(b.status, "checked");
        let until = checker.cache.lock().await.until.unwrap();
        assert!(until > Instant::now() + Duration::from_secs(86300));
        checker.cache.lock().await.until = Some(Instant::now() - Duration::from_secs(1));
        let failed = checker.check_with(|| async { Err(()) }).await;
        assert_eq!(failed.status, "unavailable");
        assert_eq!(failed.checked_at, a.checked_at);
        let until = checker.cache.lock().await.until.unwrap();
        assert!(until > Instant::now() + Duration::from_secs(3500));
        assert!(until < Instant::now() + Duration::from_secs(3601));
        assert_eq!(
            checker
                .check_with(|| async { panic!("must use cached failure") })
                .await
                .status,
            "unavailable"
        );
    }
}
