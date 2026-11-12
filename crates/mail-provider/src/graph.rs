//! Microsoft mail uses Graph, never legacy IMAP/SMTP authentication.
#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
use crate::{mime::PreparedMessage, *};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
mod batch;
mod delta;
mod send;
mod subscriptions;
pub use delta::{GraphArrivalPage, GraphCursor, GraphDeltaPage};
pub use subscriptions::GraphSubscription;

pub struct GraphProvider {
    account: String,
    mapping: FolderMapping,
    token: Arc<dyn AccessToken>,
    http: reqwest::Client,
    base: Url,
    folders: Mutex<Option<(Instant, Vec<GraphFolder>)>>,
    cooldown: Mutex<Option<Instant>>,
    retired: AtomicBool,
    immutable_ids: Mutex<Option<bool>>,
    changed: tokio::sync::Notify,
}
#[derive(Clone)]
struct GraphFolder {
    id: String,
    folder: Folder,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteFolder {
    id: String,
    display_name: String,
    child_folder_count: u32,
}
#[derive(Deserialize)]
struct Collection<T> {
    value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    next: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteMessage {
    id: String,
    is_read: bool,
    received_date_time: String,
    flag: RemoteFlag,
    #[serde(default)]
    internet_message_headers: Vec<Header>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteFlag {
    flag_status: String,
}
#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}
fn invalid() -> ProviderError {
    error(ErrorKind::Retryable, "Microsoft Graph response invalid")
}
fn segment(value: &str) -> String {
    // URL path-segment encoding, not form encoding (Graph ids can contain '+', '/', '=').
    let mut url = Url::parse("https://graph.microsoft.com/").unwrap();
    url.path_segments_mut().unwrap().push(value);
    url.path().trim_start_matches('/').to_owned()
}
impl GraphProvider {
    pub fn new(account: String, token: Arc<dyn AccessToken>, mapping: FolderMapping) -> Self {
        Self {
            account,
            mapping,
            token,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Graph HTTP client"),
            base: Url::parse("https://graph.microsoft.com/v1.0/").unwrap(),
            folders: Mutex::new(None),
            cooldown: Mutex::new(None),
            retired: AtomicBool::new(false),
            immutable_ids: Mutex::new(None),
            changed: tokio::sync::Notify::new(),
        }
    }
    /// Trusted loopback endpoint for protocol fixtures; never supplied by an HTTP caller.
    #[doc(hidden)]
    pub fn with_test_endpoint(mut self, base: Url) -> Self {
        assert!(
            base.scheme() == "http"
                && matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && base.path() == "/v1.0/"
        );
        self.base = base;
        self
    }
    fn url(&self, path: &str) -> Result<Url> {
        let url = self.base.join(path).map_err(|_| invalid())?;
        // Never send bearer tokens to a redirect, foreign nextLink or another account.
        if url.origin() != self.base.origin()
            || !(url.path().starts_with(&format!("{}me/", self.base.path()))
                || url.path() == format!("{}$batch", self.base.path())
                || url.path() == format!("{}subscriptions", self.base.path())
                || url
                    .path()
                    .starts_with(&format!("{}subscriptions/", self.base.path())))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(error(ErrorKind::InvalidInput, "invalid Graph continuation"));
        }
        Ok(url)
    }
    fn folder_resource(&self, path: &str, folder: &str, suffix: &str) -> bool {
        let Some(rest) = path.strip_prefix(&format!("{}me/", self.base.path())) else {
            return false;
        };
        if !rest
            .get(..11)
            .is_some_and(|s| s.eq_ignore_ascii_case("mailFolders"))
        {
            return false;
        }
        let rest = &rest[11..];
        let encoded = segment(folder);
        let normal = format!("/{encoded}/");
        let odata = format!("('{}')/", segment(&folder.replace('\'', "''")));
        rest.strip_prefix(&normal)
            .or_else(|| rest.strip_prefix(&odata))
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
    }
    async fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        if self.retired.load(Ordering::SeqCst) {
            return Err(error(ErrorKind::Authentication, "Graph provider retired"));
        }
        if self
            .cooldown
            .lock()
            .await
            .is_some_and(|until| until > Instant::now())
        {
            return Err(error(ErrorKind::Retryable, "Microsoft Graph throttled"));
        }
        Ok(self
            .http
            .request(method, self.url(path)?)
            .bearer_auth(self.token.access_token().await?)
            .header("Prefer", "IdType=\"ImmutableId\""))
    }
    async fn bounded(mut response: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
        if response
            .content_length()
            .is_some_and(|size| size > max_bytes as u64)
        {
            return Err(error(
                ErrorKind::Unsupported,
                "message exceeds content limit",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| error(ErrorKind::Retryable, "Microsoft Graph content interrupted"))?
        {
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(error(
                    ErrorKind::Unsupported,
                    "message exceeds content limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    async fn read_json<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
        let bytes = Self::bounded(response, 24 * 1024 * 1024).await?;
        serde_json::from_slice(&bytes).map_err(|_| invalid())
    }
    async fn checked(&self, response: reqwest::Response) -> Result<reqwest::Response> {
        if response.status().is_success() {
            return Ok(response);
        }
        if response.status() == StatusCode::BAD_REQUEST {
            let body: serde_json::Value = Self::read_json(response).await?;
            let code = body["error"]["code"].as_str().unwrap_or("");
            let stale = [
                "SyncStateNotFound",
                "InvalidDeltaToken",
                "InvalidSyncState",
                "ErrorInvalidSyncStateData",
                "resyncRequired",
            ]
            .iter()
            .any(|allowed| code.eq_ignore_ascii_case(allowed));
            return Err(error(
                if stale {
                    ErrorKind::StaleReference
                } else {
                    ErrorKind::InvalidInput
                },
                "Microsoft Graph request rejected",
            ));
        }
        let kind = match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ErrorKind::Authentication,
            StatusCode::NOT_FOUND | StatusCode::GONE => ErrorKind::StaleReference,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE => {
                let seconds = response
                    .headers()
                    .get("retry-after")
                    .and_then(|s| s.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(60)
                    .clamp(1, 86400);
                *self.cooldown.lock().await = Some(Instant::now() + Duration::from_secs(seconds));
                ErrorKind::Retryable
            }
            _ => ErrorKind::Retryable,
        };
        // Graph errors can contain addresses and message text; never expose or log them.
        Err(error(kind, "Microsoft Graph request rejected"))
    }
    async fn json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .request(Method::GET, path)
            .await?
            .send()
            .await
            .map_err(|_| error(ErrorKind::Retryable, "Microsoft Graph unavailable"))?;
        Self::read_json(self.checked(response).await?).await
    }
    pub async fn refresh_folders(&self) {
        *self.folders.lock().await = None;
    }
    async fn discovered(&self) -> Result<Vec<GraphFolder>> {
        let mut cache = self.folders.lock().await;
        if let Some((at, folders)) = cache.as_ref() {
            if at.elapsed() < Duration::from_secs(60) {
                return Ok(folders.clone());
            }
        }
        let mut roles = Vec::new();
        for (well_known, name, role) in [
            ("inbox", "INBOX", "Inbox"),
            ("sentitems", "Sent", "Sent"),
            ("drafts", "Drafts", "Drafts"),
            ("deleteditems", "Trash", "Trash"),
            ("junkemail", "Junk", "Junk"),
            ("archive", "Archive", "Archive"),
        ] {
            match self
                .json::<RemoteFolder>(&format!(
                    "me/mailFolders/{well_known}?$select=id,displayName,childFolderCount"
                ))
                .await
            {
                Ok(f) => roles.push((f.id, name.to_owned(), role.to_owned())),
                Err(e) if e.kind == ErrorKind::StaleReference => {}
                Err(e) => return Err(e),
            }
        }
        let mut pending = vec![(
            "me/mailFolders?$top=100&$select=id,displayName,childFolderCount".to_owned(),
            String::new(),
        )];
        let mut folders: Vec<GraphFolder> = Vec::new();
        let mut pages = 0;
        while let Some((path, parent)) = pending.pop() {
            pages += 1;
            if pages > 1000 {
                return Err(error(ErrorKind::Unsupported, "Graph folder limit exceeded"));
            }
            let page: Collection<RemoteFolder> = self.json(&path).await?;
            if let Some(next) = page.next {
                pending.push((next, parent.clone()));
            }
            for remote in page.value {
                if remote.id.is_empty() || folders.iter().any(|f| f.id == remote.id) {
                    return Err(invalid());
                }
                let special = roles.iter().find(|(id, _, _)| id == &remote.id);
                let name = if let Some((_, name, _)) = special {
                    name.clone()
                } else {
                    let escaped = remote.display_name.replace('%', "%25").replace('/', "%2F");
                    if parent.is_empty() {
                        format!("Folders/{escaped}")
                    } else {
                        format!("{parent}/{escaped}")
                    }
                };
                if folders.iter().any(|f| f.folder.name == name) {
                    return Err(invalid());
                }
                if remote.child_folder_count > 0 {
                    pending.push((format!("me/mailFolders/{}/childFolders?$top=100&$select=id,displayName,childFolderCount",segment(&remote.id)),name.clone()));
                }
                folders.push(GraphFolder {
                    id: remote.id,
                    folder: Folder {
                        name,
                        selectable: true,
                        attributes: special
                            .map(|(_, _, role)| vec![role.clone()])
                            .unwrap_or_default(),
                    },
                });
            }
        }
        *cache = Some((Instant::now(), folders.clone()));
        Ok(folders)
    }
    async fn folder(&self, name: &str) -> Result<GraphFolder> {
        self.discovered()
            .await?
            .into_iter()
            .find(|f| f.folder.name == name)
            .ok_or_else(|| error(ErrorKind::InvalidInput, "unknown Graph folder"))
    }
    pub fn wake(&self) {
        self.changed.notify_one();
    }
    pub async fn immutable_ids(&self) -> Result<bool> {
        let mut cached = self.immutable_ids.lock().await;
        if let Some(applied) = *cached {
            return Ok(applied);
        }
        let response = self
            .request(Method::GET, "me/messages?$top=1&$select=id")
            .await?
            .send()
            .await
            .map_err(|_| error(ErrorKind::Retryable, "Microsoft Graph unavailable"))?;
        let response = self.checked(response).await?;
        let applied = response
            .headers()
            .get("preference-applied")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.contains("ImmutableId"));
        *cached = Some(applied);
        Ok(applied)
    }
    pub async fn message_exists(&self, id: &str) -> Result<bool> {
        match self
            .json::<serde_json::Value>(&format!("me/messages/{}?$select=id", segment(id)))
            .await
        {
            Ok(body) if body["id"].as_str().is_some_and(|s| !s.is_empty()) => Ok(true),
            Ok(_) => Err(invalid()),
            Err(e) if e.kind == ErrorKind::StaleReference => Ok(false),
            Err(e) => Err(e),
        }
    }
    fn reference(&self, folder: &str, id: String) -> MessageRef {
        MessageRef {
            graph_id: Some(id),
            account_id: self.account.clone(),
            folder: folder.into(),
            uidvalidity: 1,
            uid: 1,
        }
    }
    fn message_path(&self, reference: &MessageRef) -> Result<String> {
        if reference.account_id != self.account
            || reference.uidvalidity != 1
            || reference.graph_id.as_deref().is_none_or(str::is_empty)
        {
            return Err(error(
                ErrorKind::InvalidInput,
                "Graph account/reference mismatch",
            ));
        }
        Ok(format!(
            "me/messages/{}",
            segment(reference.graph_id.as_deref().unwrap())
        ))
    }
    fn metadata(&self, folder: &str, message: RemoteMessage) -> Result<Metadata> {
        if message.id.is_empty() {
            return Err(invalid());
        }
        let date = chrono::DateTime::parse_from_rfc3339(&message.received_date_time)
            .map_err(|_| invalid())?;
        let mut headers = String::new();
        for header in message.internet_message_headers {
            if header.name.is_empty()
                || !header
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && b != b':')
            {
                return Err(invalid());
            }
            headers.push_str(&header.name);
            headers.push_str(": ");
            headers.push_str(&header.value.replace(['\r', '\n'], " "));
            headers.push_str("\r\n");
        }
        headers.push_str("\r\n");
        Ok(Metadata {
            reference: self.reference(folder, message.id.clone()),
            identity: Identity::Graph {
                account_id: self.account.clone(),
                message_id: message.id,
            },
            gmail_thread_id: None,
            labels: vec![],
            seen: message.is_read,
            starred: message.flag.flag_status == "flagged",
            size: 0,
            internal_date: date.format("%d-%b-%Y %H:%M:%S %z").to_string(),
            headers: headers.into_bytes(),
        })
    }
    async fn matches(&self, metadata: &Metadata, query: &Search) -> Result<bool> {
        if query.gmail_thread.is_some() {
            return Err(error(
                ErrorKind::Unsupported,
                "Gmail search is unavailable for Microsoft",
            ));
        }
        if query.seen.is_some_and(|s| s != metadata.seen)
            || query.starred.is_some_and(|s| s != metadata.starred)
        {
            return Ok(false);
        }
        let parsed = mail_parser::MessageParser::default()
            .parse(&metadata.headers)
            .ok_or_else(invalid)?;
        let contains = |haystack: String, needle: &Option<String>| {
            needle
                .as_ref()
                .is_none_or(|s| haystack.to_lowercase().contains(&s.to_lowercase()))
        };
        let addresses = |a: Option<&mail_parser::Address<'_>>| {
            a.map(|a| {
                a.iter()
                    .map(|a| format!("{} {}", a.name().unwrap_or(""), a.address().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
        };
        if !contains(addresses(parsed.from()), &query.from)
            || !contains(addresses(parsed.to()), &query.to)
            || !contains(parsed.subject().unwrap_or("").into(), &query.subject)
        {
            return Ok(false);
        }
        if let Some(id) = &query.conversation_id {
            let linked = parsed.headers().iter().any(|h| {
                matches!(
                    h.name,
                    mail_parser::HeaderName::MessageId
                        | mail_parser::HeaderName::References
                        | mail_parser::HeaderName::InReplyTo
                ) && (h.value.as_text() == Some(id.as_str())
                    || h.value
                        .as_text_list()
                        .is_some_and(|ids| ids.iter().any(|value| value.as_ref() == id)))
            });
            if !linked {
                return Ok(false);
            }
        }
        let received =
            chrono::DateTime::parse_from_str(&metadata.internal_date, "%d-%b-%Y %H:%M:%S %z")
                .map_err(|_| invalid())?
                .date_naive();
        for (bound, since) in [(&query.since, true), (&query.before, false)] {
            if let Some(bound) = bound {
                let date = chrono::NaiveDate::parse_from_str(bound, "%d-%b-%Y")
                    .map_err(|_| error(ErrorKind::InvalidInput, "invalid search date"))?;
                if (since && received < date) || (!since && received >= date) {
                    return Ok(false);
                }
            }
        }
        if let Some(text) = &query.text {
            let raw = self.fetch(&metadata.reference, 25 * 1024 * 1024).await?;
            let parsed = mail_parser::MessageParser::default()
                .parse(&raw)
                .ok_or_else(invalid)?;
            let text = text.to_lowercase();
            if !String::from_utf8_lossy(&metadata.headers)
                .to_lowercase()
                .contains(&text)
                && !parsed
                    .body_text(0)
                    .is_some_and(|body| body.to_lowercase().contains(&text))
                && !parsed
                    .body_html(0)
                    .is_some_and(|body| body.to_lowercase().contains(&text))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
#[async_trait]
impl MailProvider for GraphProvider {
    fn account_id(&self) -> &str {
        &self.account
    }
    fn graph(&self) -> Option<&GraphProvider> {
        Some(self)
    }
    fn retire(&self) {
        self.retired.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
    }
    async fn probe(&self) -> Result<Probe> {
        self.immutable_ids().await?;
        Ok(Probe {
            capabilities: vec!["GRAPH".into()],
            folders: self.folders().await?,
            full_search_coverage: true,
        })
    }
    async fn folders(&self) -> Result<Vec<Folder>> {
        Ok(self
            .discovered()
            .await?
            .into_iter()
            .map(|f| f.folder)
            .collect())
    }
    async fn scan(
        &self,
        folder: &str,
        query: &Search,
        continuation: Option<&PageToken>,
        limit: usize,
    ) -> Result<Page> {
        let remote = self.folder(folder).await?;
        if limit == 0 || limit > 1000 {
            return Err(error(ErrorKind::InvalidInput, "invalid Graph page size"));
        }
        let path = if let Some(token) = continuation {
            if token.account_id != self.account
                || token.folder != folder
                || token.query != *query
                || token.uidvalidity != 1
            {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "Graph continuation scope mismatch",
                ));
            }
            let path = token.graph_next.clone().ok_or_else(invalid)?;
            let url = self.url(&path)?;
            if !self.folder_resource(url.path(), &remote.id, "messages") {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "Graph continuation folder mismatch",
                ));
            }
            path
        } else {
            format!("me/mailFolders/{}/messages?$top={limit}&$select=id,isRead,flag,receivedDateTime,internetMessageHeaders",segment(&remote.id))
        };
        let page: Collection<RemoteMessage> = self.json(&path).await?;
        let mut rows = Vec::new();
        for message in page.value {
            let mut metadata = self.metadata(folder, message)?;
            if metadata.headers == b"\r\n" {
                let raw = self.fetch(&metadata.reference, 25 * 1024 * 1024).await?;
                let end = raw
                    .windows(4)
                    .position(|v| v == b"\r\n\r\n")
                    .ok_or_else(invalid)?
                    + 4;
                metadata.size = raw.len() as u32;
                metadata.headers = raw[..end].to_vec();
            }
            if self.matches(&metadata, query).await? {
                rows.push(metadata);
            }
        }
        let next = page.next.map(|next| PageToken {
            graph_next: Some(next),
            account_id: self.account.clone(),
            folder: folder.into(),
            uidvalidity: 1,
            upper_uid: 1,
            before_uid: 1,
            query: query.clone(),
            modseq: None,
        });
        // Search pages are not a stable membership snapshot and cannot authorize pruning.
        Ok(Page {
            rows,
            next,
            uidvalidity: 1,
            full_coverage: false,
        })
    }
    async fn search(
        &self,
        folder: &str,
        query: &Search,
        continuation: Option<&PageToken>,
        limit: usize,
    ) -> Result<Page> {
        self.scan(folder, query, continuation, limit).await
    }
    async fn wait_for_change(&self, _folder: &str, max_wait: Duration) -> Result<bool> {
        self.token.access_token().await?;
        let changed = tokio::time::timeout(
            max_wait.min(Duration::from_secs(60)),
            self.changed.notified(),
        )
        .await
        .is_ok();
        if self.retired.load(Ordering::SeqCst) {
            return Err(error(ErrorKind::Authentication, "Graph provider retired"));
        }
        Ok(changed)
    }
    async fn fetch_batch(&self, references: &[MessageRef], max_bytes: u32) -> Result<Vec<Vec<u8>>> {
        self.fetch_mime_batch(references, max_bytes).await
    }
    async fn fetch(&self, reference: &MessageRef, max_bytes: u32) -> Result<Vec<u8>> {
        let path = format!("{}/$value", self.message_path(reference)?);
        let response = self
            .request(Method::GET, &path)
            .await?
            .send()
            .await
            .map_err(|_| error(ErrorKind::Retryable, "Microsoft Graph unavailable"))?;
        Self::bounded(self.checked(response).await?, max_bytes as usize).await
    }
    async fn mutate(&self, reference: &MessageRef, change: Mutation) -> Result<MutationResult> {
        let (plan, destination) = self.mutation_plan(reference, &change).await?;
        let (method, path, body) = (plan.method, plan.path, plan.body.unwrap());
        let request = self.request(method, &path).await?.json(&body);
        let response = request
            .send()
            .await
            .map_err(|_| error(ErrorKind::Uncertain, "Graph mutation outcome unknown"))?;
        if response.status().is_server_error() {
            return Err(error(
                ErrorKind::Uncertain,
                "Graph mutation outcome unknown",
            ));
        }
        let response = self.checked(response).await?;
        let destination_reference = if let Some(folder) = &destination {
            #[derive(Deserialize)]
            struct Moved {
                id: String,
            }
            let moved: Moved = Self::read_json(response)
                .await
                .map_err(|_| error(ErrorKind::Uncertain, "Graph mutation outcome unknown"))?;
            if moved.id.is_empty() {
                return Err(error(
                    ErrorKind::Uncertain,
                    "Graph mutation outcome unknown",
                ));
            }
            Some(self.reference(folder, moved.id))
        } else {
            None
        };
        Ok(MutationResult {
            destination,
            destination_reference,
            requires_rescan: true,
        })
    }
    async fn submit(
        &self,
        prepared: &PreparedMessage,
        gate: &dyn SubmissionGate,
    ) -> SubmissionOutcome {
        self.submit_mime(prepared, gate).await
    }
    async fn reconcile_sent(&self, _message_id: &str, _raw: &[u8]) -> Result<SentOutcome> {
        Ok(SentOutcome::ProviderManaged)
    }
}
