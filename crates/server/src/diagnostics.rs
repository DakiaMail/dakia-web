//! Bounded, private event retention on the existing persistent data mount.
//! Only the explicitly allowlisted tracing targets are retained by main.rs.
use sha2::{Digest, Sha256};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

pub(crate) fn correlation(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn retained_target(target: &str) -> bool {
    matches!(
        target,
        "dakia_server::diagnostics"
            | "dakia_server::mutations"
            | "dakia_server::mail"
            | "dakia_server::provider_io"
    )
}

#[derive(Clone)]
pub struct Retained(Arc<Mutex<Ring>>);
struct Ring {
    directory: PathBuf,
    limit: u64,
    files: usize,
    age: Duration,
    last_pruned: std::time::Instant,
}
pub struct Event {
    ring: Retained,
    bytes: Vec<u8>,
}
impl Retained {
    pub fn open(data: &Path) -> io::Result<Self> {
        let directory = data.join("diagnostics");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let mut ring = Ring {
            directory,
            limit: 8 * 1024 * 1024,
            files: 32,
            age: Duration::from_secs(3 * 86400),
            last_pruned: std::time::Instant::now(),
        };
        ring.prune()?;
        Ok(Self(Arc::new(Mutex::new(ring))))
    }
}
impl Ring {
    fn path(&self, n: usize) -> PathBuf {
        self.directory.join(format!("mail-events.{n}.log"))
    }
    fn prune(&mut self) -> io::Result<()> {
        for n in 0..self.files {
            let p = self.path(n);
            match fs::metadata(&p) {
                Ok(m) => {
                    if SystemTime::now()
                        .duration_since(m.modified()?)
                        .unwrap_or_default()
                        > self.age
                    {
                        fs::remove_file(p)?;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        // A single unexpected oversized event must not defeat the bound.
        if bytes.len() as u64 > self.limit {
            return Ok(());
        }
        if self.last_pruned.elapsed() >= Duration::from_secs(60) {
            self.prune()?;
            self.last_pruned = std::time::Instant::now();
        }
        if fs::metadata(self.path(0)).map(|m| m.len()).unwrap_or(0) + bytes.len() as u64
            > self.limit
        {
            for n in (1..self.files).rev() {
                let from = self.path(n - 1);
                if from.exists() {
                    fs::rename(from, self.path(n))?;
                }
            }
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(self.path(0))?;
        f.write_all(bytes)?;
        f.sync_data()
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Retained {
    type Writer = Event;
    fn make_writer(&'a self) -> Event {
        Event {
            ring: self.clone(),
            bytes: Vec::new(),
        }
    }
}
impl Write for Event {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // fmt events are small. Bound buffering independently of disk rotation.
        if self.bytes.len().saturating_add(bytes.len()) > 2 * 1024 * 1024 {
            return Err(io::Error::other("diagnostic event too large"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        if let Ok(mut ring) = self.ring.0.lock() {
            if ring.append(&self.bytes).is_err() {
                // Static fallback, never a path or OS/provider error payload.
                eprintln!("mail diagnostic retention write failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::fmt::MakeWriter;
    #[test]
    fn bounded_retention_survives_reopen_and_has_private_permissions() {
        let d = tempfile::tempdir().unwrap();
        let writer = Retained::open(d.path()).unwrap();
        {
            let mut ring = writer.0.lock().unwrap();
            ring.limit = 10;
            ring.files = 3;
        }
        for n in 0..8 {
            writeln!(writer.make_writer(), "event-{n}").unwrap();
        }
        let directory = d.path().join("diagnostics");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 3);
        for entry in fs::read_dir(&directory).unwrap() {
            let m = entry.unwrap().metadata().unwrap();
            assert!(m.len() <= 10);
            assert_eq!(m.permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let before = fs::read(writer.0.lock().unwrap().path(0)).unwrap();
        drop(writer);
        let reopened = Retained::open(d.path()).unwrap();
        assert_eq!(
            fs::read(reopened.0.lock().unwrap().path(0)).unwrap(),
            before
        );
        {
            let mut ring = reopened.0.lock().unwrap();
            ring.limit = 10;
            ring.append(&[0; 11]).unwrap();
        }
        assert_eq!(
            fs::read(reopened.0.lock().unwrap().path(0)).unwrap(),
            before
        );
        {
            let mut ring = reopened.0.lock().unwrap();
            ring.age = Duration::ZERO;
            ring.prune().unwrap();
        }
        assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
    }
}
