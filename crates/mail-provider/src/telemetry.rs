//! Content-free wire accounting. Never retain commands, mailbox names or replies.
use sha2::{Digest, Sha256};
use std::time::Instant;
pub(crate) fn key(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub(crate) struct Wire {
    account_key: String,
    command: &'static str,
    started: Instant,
    bytes: usize,
    bytes_known: bool,
    pub complete: bool,
}
impl Wire {
    pub fn new(account: &str, command: &str) -> Self {
        let command = match command.split_ascii_whitespace().next() {
            Some("CAPABILITY") => "capability",
            Some("EXAMINE") => "examine",
            Some("SELECT") => "select",
            Some("ENABLE") => "enable",
            Some("LIST") => "list",
            Some("UID") => match command.split_ascii_whitespace().nth(1) {
                Some("FETCH") => "uid_fetch",
                Some("SEARCH") => "uid_search",
                Some("STORE") => "uid_store",
                Some("MOVE") => "uid_move",
                Some("COPY") => "uid_copy",
                _ => "other",
            },
            Some("APPEND") => "append",
            Some("CREATE") => "create",
            _ => "other",
        };
        Self {
            account_key: account.into(),
            command,
            started: Instant::now(),
            bytes: 0,
            bytes_known: true,
            complete: false,
        }
    }
    pub fn response(&mut self, raw: &[u8]) {
        // async-imap owns a padded allocation, potentially with later replies.
        // Reuse the protocol parser to measure this reply's consumed prefix.
        // This is telemetry only; the command's existing safety budget is unchanged.
        match imap_proto::Response::parse(raw) {
            Ok((remaining, _)) => {
                self.bytes = self.bytes.saturating_add(raw.len() - remaining.len())
            }
            Err(_) => self.bytes_known = false,
        }
    }
}
impl Drop for Wire {
    fn drop(&mut self) {
        tracing::info!(target: "dakia_server::provider_io", account_key = %self.account_key,
            command = self.command, wire_schema = 2, response_bytes = self.bytes as u64,
            response_bytes_complete = self.complete && self.bytes_known,
            elapsed_ms = self.started.elapsed().as_millis() as u64, completed = self.complete,
            "imap_wire");
    }
}

pub(crate) fn request_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    key(&format!("{at}:{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_bytes_exclude_padding_and_following_replies() {
        let first = b"* SEARCH 7 8\r\n";
        let second = b"A1 OK done\r\n";
        let mut padded = Vec::from(first.as_slice());
        padded.extend_from_slice(second);
        padded.resize(4096, 0);
        let mut wire = Wire::new("opaque", "UID SEARCH ALL");
        wire.response(&padded);
        assert_eq!(wire.bytes, first.len());
        wire.response(second);
        assert_eq!(wire.bytes, first.len() + second.len());
        assert!(wire.bytes_known);
    }
    #[test]
    fn unreadable_measurement_is_explicit_not_reported_as_exact() {
        let mut wire = Wire::new("opaque", "UID FETCH private-mailbox");
        wire.response(b"incomplete");
        assert!(!wire.bytes_known);
        assert_eq!(wire.command, "uid_fetch");
    }
}
