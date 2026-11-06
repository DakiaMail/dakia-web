#[path = "verify_mutation.rs"]
mod verify_mutation;
use crate::*;
#[path = "mutation_timing.rs"]
mod mutation_timing;
#[cfg(test)]
#[path = "imap_queue_tests.rs"]
mod queue_tests;
#[path = "imap_reconcile.rs"]
mod reconcile;
#[path = "imap_scan.rs"]
mod scan;
use reconcile::scoped_search;
#[cfg(test)]
#[path = "sent_tests.rs"]
mod sent_tests;
use async_imap::{Client, Session};
use imap_proto::rfc4315::UidSetMember;
use imap_proto::{AttributeValue, Capability, MailboxDatum, Response, ResponseCode, Status};
use std::future::Future;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::Mutex,
};
use tokio_native_tls::{TlsConnector, TlsStream};
struct Connection {
    session: Session<TlsStream<TcpStream>>,
    capabilities: Option<Vec<String>>,
    account_key: String,
}
impl std::ops::Deref for Connection {
    type Target = Session<TlsStream<TcpStream>>;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}
impl std::ops::DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}

// Checkout removes the socket from the pool. Cancellation, deadline and every
// error drop it; only explicit successful completion returns a clean stream.
struct CommandLease<'a> {
    retired: &'a std::sync::atomic::AtomicBool,
    slot: tokio::sync::MutexGuard<'a, Option<(std::time::Instant, Connection)>>,
    connection: Option<Connection>,
}
impl std::ops::Deref for CommandLease<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for CommandLease<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection.as_mut().unwrap()
    }
}
impl CommandLease<'_> {
    fn recycle(mut self) {
        if !self.retired.load(std::sync::atomic::Ordering::Acquire) {
            *self.slot = Some((std::time::Instant::now(), self.connection.take().unwrap()));
        }
    }
}

/// One command lane and one dedicated IDLE lane per account instance. The server
/// retains one instance per authorized account. Successful operations reuse one
/// authenticated session, never a cancelled or desynchronized stream.
pub struct ImapProvider {
    pub config: Config,
    credentials: Credentials,
    lane: Mutex<Option<(std::time::Instant, Connection)>>,
    idle_lane: Mutex<()>,
    reconcile_batch: std::sync::atomic::AtomicUsize,
    tls: native_tls::TlsConnector,
    retired: std::sync::atomic::AtomicBool,
    retirement: tokio::sync::watch::Sender<bool>,
}
impl ImapProvider {
    pub fn new(config: Config, credentials: Credentials) -> Result<Self> {
        let tls = native_tls::TlsConnector::new()
            .map_err(|_| error(ErrorKind::Retryable, "TLS initialization failed"))?;
        Ok(Self {
            config,
            credentials,
            lane: Mutex::new(None),
            idle_lane: Mutex::new(()),
            reconcile_batch: std::sync::atomic::AtomicUsize::new(50),
            tls,
            retired: std::sync::atomic::AtomicBool::new(false),
            retirement: tokio::sync::watch::channel(false).0,
        })
    }
    /// Private server root, not an accept-invalid-certificates switch.
    pub fn with_root(mut self, root: native_tls::Certificate) -> Result<Self> {
        self.tls = native_tls::TlsConnector::builder()
            .add_root_certificate(root)
            .build()
            .map_err(|_| error(ErrorKind::InvalidInput, "invalid trust root"))?;
        Ok(self)
    }
    async fn bounded<T>(&self, f: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::time::timeout(self.config.timeout, f)
            .await
            .map_err(|_| error(ErrorKind::Retryable, "IMAP operation deadline exceeded"))?
    }
    async fn connect(&self) -> Result<Connection> {
        let endpoint = &self.config.imap;
        let tcp = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
            .await
            .map_err(io_error)?;
        let connector = TlsConnector::from(self.tls.clone());
        let (stream, greeted) = match endpoint.security {
            Security::Tls => (
                connector
                    .connect(&endpoint.host, tcp)
                    .await
                    .map_err(|_| error(ErrorKind::Retryable, "IMAP TLS verification failed"))?,
                false,
            ),
            Security::StartTls => {
                let mut reader = BufReader::new(tcp);
                let greeting = handshake_line(&mut reader).await?;
                if !greeting.to_ascii_uppercase().starts_with("* OK ") {
                    return Err(error(ErrorKind::Retryable, "invalid IMAP greeting"));
                }
                reader
                    .get_mut()
                    .write_all(b"DSTART STARTTLS\r\n")
                    .await
                    .map_err(io_error)?;
                reader.get_mut().flush().await.map_err(io_error)?;
                loop {
                    let line = handshake_line(&mut reader).await?;
                    if line.starts_with("DSTART ") {
                        if !line.to_ascii_uppercase().starts_with("DSTART OK ") {
                            return Err(error(
                                ErrorKind::Unsupported,
                                "server refused required STARTTLS",
                            ));
                        }
                        break;
                    }
                }
                if !reader.buffer().is_empty() {
                    return Err(error(
                        ErrorKind::Retryable,
                        "unexpected data before TLS upgrade",
                    ));
                }
                (
                    connector
                        .connect(&endpoint.host, reader.into_inner())
                        .await
                        .map_err(|_| {
                            error(ErrorKind::Retryable, "IMAP STARTTLS verification failed")
                        })?,
                    true,
                )
            }
        };
        let mut client = Client::new(stream);
        if !greeted {
            let greeting = client
                .read_response()
                .await
                .map_err(io_error)?
                .ok_or_else(|| error(ErrorKind::Retryable, "missing IMAP greeting"))?;
            if !matches!(
                greeting.parsed(),
                Response::Data {
                    status: Status::Ok,
                    ..
                }
            ) {
                return Err(error(ErrorKind::Retryable, "invalid IMAP greeting"));
            }
        }
        client
            .login(&self.credentials.username, &self.credentials.password)
            .await
            .map_err(|(e, _)| match e {
                async_imap::error::Error::No(_) => {
                    error(ErrorKind::Authentication, "IMAP authentication failed")
                }
                other => imap_error(other),
            })
            .map(|session| Connection {
                session,
                capabilities: None,
                account_key: crate::telemetry::key(&self.config.account_id),
            })
    }
    async fn command(&self) -> Result<CommandLease<'_>> {
        let mut slot = self.lane.lock().await;
        if self.retired.load(std::sync::atomic::Ordering::Acquire) {
            slot.take();
            return Err(error(
                ErrorKind::Authentication,
                "account provider replaced",
            ));
        }
        let connection = match slot.take() {
            Some((at, connection)) if at.elapsed() < std::time::Duration::from_secs(60) => {
                tracing::info!(target: "dakia_server::provider_io", account_key = %crate::telemetry::key(&self.config.account_id), reused = true, "imap_connection");
                connection
            }
            _ => {
                tracing::info!(target: "dakia_server::provider_io", account_key = %crate::telemetry::key(&self.config.account_id), reused = false, "imap_connection");
                self.connect().await?
            }
        };
        Ok(CommandLease {
            retired: &self.retired,
            slot,
            connection: Some(connection),
        })
    }
    pub fn retire(&self) {
        self.retirement.send_replace(true);
        self.retired
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut slot) = self.lane.try_lock() {
            slot.take();
        }
    }
    fn coverage(&self) -> bool {
        self.config.kind != ProviderKind::Gmail || self.config.full_imap_exposure_confirmed
    }
    pub async fn probe(&self) -> Result<Probe> {
        self.bounded(async {
            let mut s = self.command().await?;
            let capabilities = capabilities(&mut s).await?;
            if self.config.kind == ProviderKind::Gmail && !has(&capabilities, "X-GM-EXT-1") {
                return Err(error(
                    ErrorKind::Unsupported,
                    "Gmail extensions required for Gmail identity",
                ));
            }
            let folders = folders(&mut s).await?;
            s.recycle();
            Ok(Probe {
                capabilities,
                folders,
                full_search_coverage: self.coverage(),
            })
        })
        .await
    }
    /// A separate bounded connection. Unsupported IDLE tells the scheduler to poll.
    /// Returns a hint to rescan, never treats unsolicited data as durable sync.
    pub async fn wait_for_change(
        &self,
        folder: &str,
        max_wait: std::time::Duration,
    ) -> Result<bool> {
        let mut retired = self.retirement.subscribe();
        if *retired.borrow() {
            return Err(error(
                ErrorKind::Authentication,
                "account provider replaced",
            ));
        }
        tokio::select! {
            biased;
            _ = retired.changed() => Err(error(ErrorKind::Authentication, "account provider replaced")),
            result = self.wait_for_change_inner(folder, max_wait) => result,
        }
    }
    async fn wait_for_change_inner(
        &self,
        folder: &str,
        max_wait: std::time::Duration,
    ) -> Result<bool> {
        quote(folder)?;
        if max_wait.is_zero() || max_wait > std::time::Duration::from_secs(28 * 60) {
            return Err(error(
                ErrorKind::InvalidInput,
                "IDLE wait must be positive and at most 28 minutes",
            ));
        }
        let _guard = self
            .bounded(async { Ok(self.idle_lane.lock().await) })
            .await?;
        let mut idle = self
            .bounded(async {
                let mut s = self.connect().await?;
                if !has(&capabilities(&mut s).await?, "IDLE") {
                    return Err(error(
                        ErrorKind::Unsupported,
                        "IDLE unsupported; use polling",
                    ));
                }
                select(&mut s, folder, true).await?;
                let mut idle = s.session.idle();
                idle.init().await.map_err(imap_error)?;
                tracing::info!(target: "dakia_server::provider_io", operation = "idle_wait", "provider_io");
                Ok(idle)
            })
            .await?;
        let response = {
            let (wait, _stop) = idle.wait_with_timeout(max_wait);
            tokio::time::timeout(max_wait, wait).await
        };
        let changed = match response {
            Ok(Ok(async_imap::extensions::idle::IdleResponse::NewData(_))) => true,
            Ok(Ok(async_imap::extensions::idle::IdleResponse::Timeout)) | Err(_) => false,
            _ => return Err(error(ErrorKind::Retryable, "IDLE connection interrupted")),
        };
        self.bounded(async {
            idle.done().await.map_err(imap_error)?;
            Ok(())
        })
        .await?;
        Ok(changed)
    }
    /// Full metadata reconciliation is the portable delta fallback, not just a
    /// new-UID scan: it refreshes flags and discovers removals too. Persist page
    /// tokens and a scan generation; prune absent refs only after a full success.
    /// A UIDVALIDITY reset invalidates folder refs, never Dakia-owned draft state.
    pub async fn scan(
        &self,
        folder: &str,
        query: &Search,
        continuation: Option<&PageToken>,
        limit: usize,
    ) -> Result<Page> {
        if !(1..=200).contains(&limit) {
            return Err(error(ErrorKind::InvalidInput, "page size must be 1..200"));
        }
        quote(folder)?;
        let mut search = query.imap_query()?;
        if !search.is_ascii() {
            search = format!("CHARSET UTF-8 {search}");
        }
        self.bounded(async {
            let mut s = self.command().await?;
            let validity = select(&mut s, folder, true).await?;
            if let Some(t) = continuation {
                if t.account_id != self.config.account_id || t.folder != folder || t.query != *query
                {
                    return Err(error(
                        ErrorKind::InvalidInput,
                        "continuation does not match search",
                    ));
                }
                if t.uidvalidity != validity {
                    return Err(error(
                        ErrorKind::StaleReference,
                        "UIDVALIDITY changed; restart folder scan",
                    ));
                }
            }
            let mut uids = match scoped_search(&search, continuation) {
                Some(scoped) => search_uids(&mut s, &scoped).await?,
                None => Vec::new(),
            };
            uids.sort_unstable_by(|a, b| b.cmp(a));
            uids.dedup();
            let upper = continuation
                .map(|t| t.upper_uid)
                .unwrap_or_else(|| uids.first().copied().unwrap_or(0));
            uids.retain(|u| *u <= upper && continuation.is_none_or(|t| *u < t.before_uid));
            let total = uids.len();
            uids.truncate(limit);
            let (rows, consumed) = self.metadata(&mut s, folder, validity, &uids).await?;
            let next = if consumed < total {
                uids.get(consumed.saturating_sub(1)).map(|uid| PageToken {
                    account_id: self.config.account_id.clone(),
                    folder: folder.into(),
                    uidvalidity: validity,
                    upper_uid: upper,
                    before_uid: *uid,
                    query: query.clone(),
                    modseq: None,
                })
            } else {
                None
            };
            tracing::info!(target: "dakia_server::provider_io", operation = "scan_result", rows = rows.len(), has_next = next.is_some(), "provider_io");
            s.recycle();
            Ok(Page {
                rows,
                next,
                uidvalidity: validity,
                full_coverage: self.coverage(),
            })
        })
        .await
    }
    fn check_account(&self, r: &MessageRef) -> Result<()> {
        if r.account_id != self.config.account_id || r.uid == 0 {
            return Err(error(
                ErrorKind::InvalidInput,
                "reference belongs to another account or invalid UID",
            ));
        }
        quote(&r.folder)?;
        Ok(())
    }
    async fn select_ref(&self, s: &mut Connection, r: &MessageRef) -> Result<()> {
        if select(s, &r.folder, false).await? != r.uidvalidity {
            return Err(error(ErrorKind::StaleReference, "UIDVALIDITY changed"));
        }
        if !search_uids(s, &format!("UID {}", r.uid))
            .await?
            .contains(&r.uid)
        {
            return Err(error(ErrorKind::StaleReference, "message no longer exists"));
        }
        Ok(())
    }
    pub async fn fetch(&self, r: &MessageRef, max_bytes: u32) -> Result<Vec<u8>> {
        self.check_account(r)?;
        if max_bytes == 0 || max_bytes > 64 * 1024 * 1024 {
            return Err(error(
                ErrorKind::InvalidInput,
                "content limit must be 1..64 MiB",
            ));
        }
        self.bounded(async {
            let mut s = self.command().await?;
            self.select_ref(&mut s, r).await?;
            let uid = r.uid.to_string();
            let sizes = fetches(&mut s, &uid, "UID RFC822.SIZE").await?;
            let mut target_sizes = sizes
                .iter()
                .filter(|f| f.uid == Some(r.uid))
                .filter_map(|f| f.size);
            let size = target_sizes
                .next()
                .ok_or_else(|| error(ErrorKind::StaleReference, "message size unavailable"))?;
            if target_sizes.any(|other| other != size) {
                return Err(error(ErrorKind::Retryable, "conflicting message sizes"));
            }
            if size > max_bytes {
                // The tagged size response is complete; rejection is local policy.
                s.recycle();
                return Err(error(
                    ErrorKind::InvalidInput,
                    "message exceeds content limit",
                ));
            }
            let bodies = fetches(
                &mut s,
                &uid,
                &format!("UID BODY.PEEK[]<0.{}>", u64::from(max_bytes) + 1),
            )
            .await?;
            let mut target_bodies = bodies
                .into_iter()
                .filter(|f| f.uid == Some(r.uid) && f.full_body && f.body_offset.unwrap_or(0) == 0)
                .filter_map(|f| f.body);
            let body = target_bodies
                .next()
                .ok_or_else(|| error(ErrorKind::StaleReference, "message body unavailable"))?;
            if body.len() > max_bytes as usize {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "message exceeds content limit",
                ));
            }
            if target_bodies.any(|other| other != body) {
                return Err(error(
                    ErrorKind::Retryable,
                    "incomplete or conflicting message body",
                ));
            }
            if body.len() != size as usize {
                // Gmail can disagree with its advertised RFC822.SIZE. Never
                // assume a short literal is complete: verify EOF at the exact
                // returned offset, within the original byte/deadline budget.
                if self.config.kind != ProviderKind::Gmail || body.is_empty() {
                    return Err(error(ErrorKind::Retryable, "incomplete or conflicting message body"));
                }
                let eof = fetches(&mut s, &uid, &format!("UID BODY.PEEK[]<{}.1>", body.len())).await?;
                let mut witnesses = eof.iter().filter(|f| f.uid == Some(r.uid) && f.full_body && f.body_offset == Some(body.len() as u32));
                let verified = witnesses.next().is_some_and(|f| f.body.as_ref().is_some_and(Vec::is_empty))
                    && witnesses.all(|f| f.body.as_ref().is_some_and(Vec::is_empty));
                tracing::info!(target: "dakia_server::provider_io", operation = "body_size_check", advertised_bytes = size as u64, body_bytes = body.len() as u64, outcome = if verified { "eof_verified" } else { "incomplete" }, "provider_io");
                if !verified {
                    return Err(error(ErrorKind::Retryable, "incomplete or conflicting message body"));
                }
            }
            s.recycle();
            Ok(body)
        })
        .await
    }
    pub async fn mutate(&self, r: &MessageRef, mutation: Mutation) -> Result<MutationResult> {
        self.check_account(r)?;
        let spam = matches!(mutation, Mutation::Spam);
        let mut timing = mutation_timing::MutationTiming::new(self.config.timeout);
        let mut write_started = false;
        let result=self.bounded(async {
            let mut s=self.command().await?;
            timing.stage("select_reference");
            self.select_ref(&mut s,r).await?;
            timing.stage("capabilities");
            let caps=capabilities(&mut s).await?;let mut destination_reference=None;
            let destination=match mutation {
                Mutation::Seen(v)|Mutation::Starred(v)=>{
                    let flag=if matches!(mutation,Mutation::Seen(_)){"\\Seen"}else{"\\Flagged"};
                    write_started = true;
                    timing.write();
                    checked(&mut s,&format!("UID STORE {} {}FLAGS.SILENT ({flag})",r.uid,if v{"+"}else{"-"})).await?;
                    timing.acknowledged();None
                }
                Mutation::Archive if self.config.kind==ProviderKind::Gmail && r.folder.eq_ignore_ascii_case("INBOX") && has(&caps,"MOVE")=>{
                    // Gmail's discovered All Mail mailbox is its archive. MOVE
                    // from INBOX removes that membership only, without a global
                    // EXPUNGE or replaying a possibly applied label command.
                    timing.stage("folder_discovery");
                    let dest=self.special_folder(&mut s,"Archive").await?;
                    if dest.eq_ignore_ascii_case(&r.folder) {return Err(error(ErrorKind::InvalidInput,"source and destination identical"));}
                    write_started = true;
                    timing.write();
                    destination_reference=transfer(&mut s,r,&dest,"MOVE").await?;
                    timing.acknowledged();
                    if search_uids(&mut s,&format!("UID {}",r.uid)).await?.contains(&r.uid) {
                        return Err(error(ErrorKind::Uncertain,"archive did not remove Inbox; reconcile before retry"));
                    }
                    Some(dest)
                }
                Mutation::Archive if self.config.kind==ProviderKind::Gmail=>{
                    write_started = true;
                    timing.write();
                    checked(&mut s,&format!("UID STORE {} -X-GM-LABELS ({})",r.uid,quote("\\Inbox")?)).await?;
                    timing.acknowledged();
                    let remaining=fetches(&mut s,&r.uid.to_string(),"UID X-GM-LABELS").await?;
                    if remaining.iter().any(|f| f.uid==Some(r.uid) && f.labels.iter().any(|label|label.trim_start_matches('\\').eq_ignore_ascii_case("Inbox"))) {
                        return Err(error(ErrorKind::Uncertain,"archive did not remove Inbox; reconcile before retry"));
                    }
                    None
                }
                other=>{
                    timing.stage("folder_discovery");
                    let(dest,copy)=match other {
                        Mutation::Archive=>(self.special_folder(&mut s,"Archive").await?,false),
                        Mutation::Trash=>(self.special_folder(&mut s,"Trash").await?,false),
                        Mutation::Spam=>(self.special_folder(&mut s,"Junk").await?,false),
                        Mutation::Move(d)=>(d,false),Mutation::Copy(d)=>(d,true),_=>unreachable!(),
                    };
                    quote(&dest)?;if dest==r.folder {return Err(error(ErrorKind::InvalidInput,"source and destination identical"));}
                    if copy || has(&caps,"MOVE") {
                        write_started = true;
                    timing.write();
                        destination_reference=transfer(&mut s,r,&dest,if copy{"COPY"}else{"MOVE"}).await?;
                        timing.acknowledged();
                    }
                    else if has(&caps,"UIDPLUS") {
                        write_started = true;
                    timing.write();
                        destination_reference=transfer(&mut s,r,&dest,"COPY").await?;
                        timing.acknowledged();
                        timing.stage("move_cleanup");
                        // Once COPY succeeds any later failure is partial/uncertain,
                        // including an explicit STORE/EXPUNGE rejection. Do not copy twice.
                        let cleanup=async {
                            checked(&mut s,&format!("UID STORE {} +FLAGS.SILENT (\\Deleted)",r.uid)).await?;
                            checked(&mut s,&format!("UID EXPUNGE {}",r.uid)).await?;Ok::<_,ProviderError>(())
                        }.await;
                        if cleanup.is_err(){return Err(error(ErrorKind::Uncertain,"copy succeeded but move cleanup incomplete; reconcile before retry"));}
                    } else {return Err(error(ErrorKind::Unsupported,"safe move requires MOVE or UIDPLUS; no global EXPUNGE fallback"));}
                    Some(dest)
                }
            };
            if spam {
                timing.stage("spam_verification");
                // Never replay an uncertain move. Verify both UID epochs and
                // exact destination membership, not just tagged acceptance.
                if search_uids(&mut s,&format!("UID {}",r.uid)).await?.contains(&r.uid) {
                    return Err(error(ErrorKind::Uncertain,"spam move source still present; reconcile before retry"));
                }
                let dest = destination_reference.as_ref().ok_or_else(|| error(ErrorKind::Uncertain,"spam destination UID unknown; reconcile before retry"))?;
                self.select_ref(&mut s,dest).await.map_err(|_| error(ErrorKind::Uncertain,"spam destination readback failed; reconcile before retry"))?;
                if !search_uids(&mut s,&format!("UID {}",dest.uid)).await?.contains(&dest.uid) {
                    return Err(error(ErrorKind::Uncertain,"spam destination missing; reconcile before retry"));
                }
            }
            s.recycle();
            Ok(MutationResult {requires_rescan:destination.is_some(),destination,destination_reference})
        }).await;
        let result = result.map_err(|e| {
            if write_started && e.kind == ErrorKind::Retryable {
                // Preserve the safe deadline/transport reason, but never replay.
                ProviderError {
                    kind: ErrorKind::Uncertain,
                    message: e.message,
                }
            } else {
                e
            }
        });
        timing.finish(match &result {
            Ok(_) => "completed",
            Err(e) if e.kind == ErrorKind::Uncertain => "unknown",
            Err(_) => "failed",
        });
        result
    }
    async fn special_folder(&self, s: &mut Connection, role: &str) -> Result<String> {
        let mapped = match role {
            "Sent" => &self.config.folders.sent,
            "Archive" => &self.config.folders.archive,
            "Trash" => &self.config.folders.trash,
            // Junk requires advertised special-use, never a guessed name or Trash.
            _ => &None,
        };
        if let Some(name) = mapped {
            quote(name)?;
            return Ok(name.clone());
        }
        let names = folders(s).await?;
        let role = if role == "Archive" && self.config.kind == ProviderKind::Gmail {
            "All"
        } else {
            role
        };
        let found: Vec<_> = names
            .iter()
            .filter(|n| n.selectable && has(&n.attributes, role))
            .collect();
        if found.len() != 1 {
            return Err(error(
                ErrorKind::Unsupported,
                "special-use folder missing/ambiguous; explicit mapping required",
            ));
        }
        Ok(found[0].name.clone())
    }
    /// Only after persisted SMTP acceptance. A filing failure never resubmits mail.
    pub async fn reconcile_sent(&self, message_id: &str, raw: &[u8]) -> Result<SentOutcome> {
        if self.config.kind == ProviderKind::Gmail
            && self
                .config
                .smtp
                .host
                .trim_end_matches('.')
                .eq_ignore_ascii_case("smtp.gmail.com")
        {
            return Ok(SentOutcome::ProviderManaged);
        }
        let quoted = quote(message_id)?;
        let expected = message_id.trim_start_matches('<').trim_end_matches('>');
        let parsed = mail_parser::MessageParser::default()
            .parse(raw)
            .ok_or_else(|| error(ErrorKind::InvalidInput, "invalid MIME"))?;
        if parsed.message_id() != Some(expected) {
            return Err(error(
                ErrorKind::InvalidInput,
                "Sent Message-ID does not match immutable MIME",
            ));
        }
        self.bounded(async {
            let mut s = self.command().await?;
            let names = folders(&mut s).await?;
            let candidates = sent_candidates(&names, self.config.folders.sent.as_deref())?;
            let mut evidence = Vec::new();
            // Reconcile every plausible Sent view before APPEND. Changing the
            // chosen folder must not duplicate a previously filed identity.
            for sent in &candidates {
                let selected = checked(&mut s, &format!("EXAMINE {}", quote(sent)?)).await?;
                let count = selected
                    .exists
                    .filter(|_| selected.uidvalidity.is_some())
                    .ok_or_else(|| error(ErrorKind::Unsupported, "incomplete Sent evidence"))?;
                evidence.push((sent.clone(), count));
                let existing = search_uids(&mut s, &format!("HEADER Message-ID {quoted}")).await?;
                // SEARCH HEADER matches substrings. Check exact IDs before assuming a
                // previously ambiguous APPEND succeeded. App IDs are immutable/unique.
                for chunk in existing.chunks(100) {
                    let seq = chunk
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    for f in fetches(
                        &mut s,
                        &seq,
                        "UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]<0.8192>",
                    )
                    .await?
                    {
                        if let Some(header) = f.body {
                            if mail_parser::MessageParser::default()
                                .parse(&header)
                                .and_then(|m| m.message_id().map(str::to_owned))
                                .as_deref()
                                == Some(expected)
                            {
                                s.recycle();
                                return Ok(SentOutcome::AlreadyPresent);
                            }
                        }
                    }
                }
            }
            let sent = choose_sent(&evidence, self.config.folders.sent.as_deref())?;
            match s.append(&sent, Some("(\\Seen)"), None, raw).await {
                Ok(()) => {
                    s.recycle();
                    Ok(SentOutcome::Appended)
                }
                Err(_) => Ok(SentOutcome::FilingUnknown),
            }
        })
        .await
        .map_err(|e| {
            if e.kind == ErrorKind::Retryable {
                error(
                    ErrorKind::Uncertain,
                    "Sent filing interrupted; reconcile by immutable Message-ID before retry",
                )
            } else {
                e
            }
        })
    }
}
// Roles outrank conventional names. Never CREATE or choose an arbitrary folder.
fn sent_candidates(names: &[Folder], explicit: Option<&str>) -> Result<Vec<String>> {
    let mut candidates: Vec<String> = names
        .iter()
        .filter(|f| f.selectable && has(&f.attributes, "Sent"))
        .map(|f| f.name.clone())
        .collect();
    if candidates.is_empty() {
        candidates = names
            .iter()
            .filter(|f| f.selectable && sent_name_rank(&f.name) < 4)
            .map(|f| f.name.clone())
            .collect();
    }
    if let Some(name) = explicit {
        quote(name)?;
        if !names.iter().any(|f| f.selectable && f.name == name) {
            return Err(error(
                ErrorKind::Unsupported,
                "configured Sent mailbox unavailable",
            ));
        }
        candidates.push(name.into());
    }
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() || candidates.len() > 32 {
        return Err(error(
            ErrorKind::Unsupported,
            "no bounded selectable Sent candidates",
        ));
    }
    Ok(candidates)
}
fn sent_name_rank(name: &str) -> u8 {
    match name
        .rsplit(['/', '.'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
        .as_str()
    {
        "sent" => 0,
        "sent messages" => 1,
        "sent items" => 2,
        "sent mail" => 3,
        _ => 4,
    }
}
fn choose_sent(evidence: &[(String, u32)], explicit: Option<&str>) -> Result<String> {
    if let Some(name) = explicit {
        return Ok(name.into());
    }
    // Existing Sent use wins, then a deterministic empty/equal-folder fallback.
    // Counts are mailbox metadata, not inspection of unrelated message bodies.
    evidence
        .iter()
        .min_by_key(|(name, count)| {
            (
                std::cmp::Reverse(*count),
                sent_name_rank(name),
                name.as_str(),
            )
        })
        .map(|(name, _)| name.clone())
        .ok_or_else(|| error(ErrorKind::Unsupported, "no Sent mailbox evidence"))
}
async fn handshake_line(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut line = String::new();
    (&mut *reader)
        .take(8193)
        .read_line(&mut line)
        .await
        .map_err(io_error)?;
    if line.is_empty() || line.len() > 8192 || !line.ends_with("\r\n") {
        return Err(error(
            ErrorKind::Retryable,
            "invalid or oversized STARTTLS response",
        ));
    }
    Ok(line)
}
fn io_error(_: std::io::Error) -> ProviderError {
    error(ErrorKind::Retryable, "IMAP transport interrupted")
}
fn imap_error(e: async_imap::error::Error) -> ProviderError {
    match e {
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => {
            error(ErrorKind::Unsupported, "IMAP command rejected")
        }
        _ => error(ErrorKind::Retryable, "IMAP protocol/transport failed"),
    }
}
fn has(values: &[String], value: &str) -> bool {
    values.iter().any(|v| v.eq_ignore_ascii_case(value))
}
fn throttle_response(information: &str) -> bool {
    let upper = information.to_ascii_uppercase();
    [
        "[LIMIT]",
        "[UNAVAILABLE]",
        "[RATE]",
        "RATE LIMIT EXCEEDED",
        "TOO MANY SIMULTANEOUS CONNECTIONS",
    ]
    .iter()
    .any(|prefix| upper.starts_with(prefix))
}
/// async-imap 0.12 streamed LIST/FETCH/STORE discard tagged NO/BAD and can
/// terminate at EOF. Use its supported command/decoded-response API and require
/// matching tagged OK on EVERY command. No homegrown IMAP parser or codec fork.
async fn checked(s: &mut Connection, command: &str) -> Result<CommandOutput> {
    let mut metric = crate::telemetry::Wire::new(&s.account_key, command);
    let tag = s.run_command(command).await.map_err(imap_error)?;
    let mut responses = CommandOutput::default();
    let mut bytes = 0usize;
    let budget = if command.contains("BODY.PEEK[]") {
        65 * 1024 * 1024
    } else {
        8 * 1024 * 1024
    };
    loop {
        let response = s.read_response().await.map_err(io_error)?.ok_or_else(|| {
            error(
                ErrorKind::Retryable,
                "IMAP connection closed before tagged completion",
            )
        })?;
        bytes = bytes.saturating_add(response.borrow_owner().len());
        metric.response(response.borrow_owner());
        if bytes > budget {
            return Err(error(
                ErrorKind::Unsupported,
                "IMAP response budget exceeded",
            ));
        }
        if response.request_id().is_some_and(|id| id != &tag) {
            return Err(error(
                ErrorKind::Retryable,
                "unexpected IMAP completion tag",
            ));
        }
        if response.request_id() == Some(&tag) {
            if !matches!(
                response.parsed(),
                Response::Done {
                    status: Status::Ok,
                    ..
                }
            ) {
                let throttled = match response.parsed() {
                    Response::Done {
                        outcome,
                        status: Status::No,
                        ..
                    } => outcome
                        .information
                        .as_deref()
                        .is_some_and(throttle_response),
                    _ => false,
                };
                return Err(if throttled {
                    error(ErrorKind::Retryable, "IMAP provider throttled")
                } else {
                    error(ErrorKind::Unsupported, "IMAP command rejected")
                });
            }
            responses.capture(response.parsed());
            metric.complete = true;
            return Ok(responses);
        }
        if matches!(
            response.parsed(),
            Response::Data {
                status: Status::Bye,
                ..
            }
        ) {
            return Err(error(ErrorKind::Retryable, "IMAP server closed session"));
        }
        responses.capture(response.parsed());
    }
}
async fn capabilities(s: &mut Connection) -> Result<Vec<String>> {
    if let Some(cached) = &s.capabilities {
        return Ok(cached.clone());
    }
    let caps = checked(s, "CAPABILITY").await?.capabilities;
    s.capabilities = Some(caps.clone());
    Ok(caps)
}
async fn folders(s: &mut Connection) -> Result<Vec<Folder>> {
    Ok(checked(s, "LIST \"\" *").await?.folders)
}
async fn select(s: &mut Connection, folder: &str, readonly: bool) -> Result<u32> {
    checked(
        s,
        &format!(
            "{} {}",
            if readonly { "EXAMINE" } else { "SELECT" },
            quote(folder)?
        ),
    )
    .await?
    .uidvalidity
    .ok_or_else(|| error(ErrorKind::Unsupported, "missing UIDVALIDITY"))
}
async fn search_uids(s: &mut Connection, query: &str) -> Result<Vec<u32>> {
    Ok(checked(s, &format!("UID SEARCH {query}")).await?.uids)
}
#[derive(Default)]
struct Fetched {
    uid: Option<u32>,
    size: Option<u32>,
    internal_date: Option<String>,
    flags: Vec<String>,
    has_flags: bool,
    body: Option<Vec<u8>>,
    full_body: bool,
    body_offset: Option<u32>,
    gmail_id: Option<u64>,
    gmail_thread: Option<u64>,
    labels: Vec<String>,
    has_labels: bool,
}
async fn fetches(s: &mut Connection, sequence: &str, fields: &str) -> Result<Vec<Fetched>> {
    Ok(checked(s, &format!("UID FETCH {sequence} ({fields})"))
        .await?
        .fetches)
}
async fn transfer(
    s: &mut Connection,
    r: &MessageRef,
    dest: &str,
    verb: &str,
) -> Result<Option<MessageRef>> {
    let output = checked(s, &format!("UID {verb} {} {}", r.uid, quote(dest)?)).await?;
    Ok(output.copyuid.and_then(|(validity, src, uid)| {
        if src == r.uid {
            Some(MessageRef {
                account_id: r.account_id.clone(),
                folder: dest.into(),
                uidvalidity: validity,
                uid,
            })
        } else {
            None
        }
    }))
}
fn single_uid(set: &[UidSetMember]) -> Option<u32> {
    match set {
        [UidSetMember::Uid(v)] => Some(*v),
        [UidSetMember::UidRange(v)] if v.start() == v.end() => Some(*v.start()),
        _ => None,
    }
}
#[derive(Default)]
struct CommandOutput {
    capabilities: Vec<String>,
    folders: Vec<Folder>,
    uidvalidity: Option<u32>,
    highest_modseq: Option<u64>,
    no_modseq: bool,
    exists: Option<u32>,
    uids: Vec<u32>,
    fetches: Vec<Fetched>,
    copyuid: Option<(u32, u32, u32)>,
}
impl CommandOutput {
    fn capture(&mut self, response: &Response<'_>) {
        match response {
            Response::Capabilities(values) => {
                for cap in values {
                    self.capabilities.push(match cap {
                        Capability::Imap4rev1 => "IMAP4rev1".into(),
                        Capability::Auth(v) => format!("AUTH={v}"),
                        Capability::Atom(v) => v.to_string(),
                    });
                }
            }
            Response::MailboxData(MailboxDatum::List(data)) => self.folders.push(Folder {
                name: data.name.to_string(),
                selectable: !data
                    .name_attributes
                    .contains(&imap_proto::NameAttribute::NoSelect),
                attributes: data
                    .name_attributes
                    .iter()
                    .map(|a| match a {
                        imap_proto::NameAttribute::Extension(value) => {
                            value.trim_start_matches('\\').to_owned()
                        }
                        other => format!("{other:?}"),
                    })
                    .collect(),
            }),
            Response::MailboxData(MailboxDatum::Exists(value)) => self.exists = Some(*value),
            Response::MailboxData(MailboxDatum::Search(values)) => self.uids.extend(values),
            Response::Data { outcome, .. } | Response::Done { outcome, .. } => {
                // Unknown response codes remain in imap-proto's information.
                self.no_modseq |= outcome
                    .information
                    .as_deref()
                    .is_some_and(|v| v.to_ascii_uppercase().starts_with("[NOMODSEQ]"));
                match &outcome.code {
                    Some(ResponseCode::UidValidity(v)) => self.uidvalidity = Some(*v),
                    Some(ResponseCode::HighestModSeq(v)) => self.highest_modseq = Some(*v),

                    Some(ResponseCode::CopyUid(v, src, dst)) => {
                        if let (Some(src), Some(dst)) = (single_uid(src), single_uid(dst)) {
                            self.copyuid = Some((*v, src, dst));
                        }
                    }
                    _ => {}
                }
            }
            Response::Fetch(_, attrs) => {
                let mut f = Fetched::default();
                for a in attrs {
                    match a {
                        AttributeValue::Uid(v) => f.uid = Some(*v),
                        AttributeValue::Rfc822Size(v) => f.size = Some(*v),
                        AttributeValue::InternalDate(v) => f.internal_date = Some(v.to_string()),
                        AttributeValue::Flags(v) => {
                            f.has_flags = true;
                            f.flags = v.iter().map(|s| s.to_string()).collect()
                        }
                        AttributeValue::BodySection {
                            section,
                            index,
                            data: Some(v),
                        } => {
                            f.body = Some(v.to_vec());
                            f.full_body = section.is_none();
                            f.body_offset = *index;
                        }
                        AttributeValue::GmailMsgId(v) => f.gmail_id = Some(*v),
                        AttributeValue::GmailThrId(v) => f.gmail_thread = Some(*v),
                        AttributeValue::GmailLabels(v) => {
                            f.has_labels = true;
                            f.labels = v.iter().map(|s| s.to_string()).collect()
                        }
                        _ => {}
                    }
                }
                self.fetches.push(f);
            }
            _ => {}
        }
    }
}
