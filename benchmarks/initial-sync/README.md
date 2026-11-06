# Initial mailbox sync comparison

A deterministic TLS IMAP fixture with 50,000 distinct messages in INBOX.
Each has a unique Message-ID, roughly 260 bytes of headers and about 1.7 KiB of text.
The fixture counts connections, commands, application bytes and header/body
rows. It rejects mutation commands. No real accounts or credentials are used.

Completion requires **all 50,000 bodies downloaded and indexed**, plus all
message metadata and folder memberships reconciled. Dakia's harness verifies
exact UID coverage, subject/UID correspondence, unread state, each body's
unique token and FTS results matching all 50,000 messages. It uses
AccountProvider, MailRuntime, the production executor and SQLite. Scheduler
waits count. `--engine-only` in run_dakia.py bypasses admission waits for
diagnosis and must be reported separately.

pl_mail uses its real ImapMailboxImporter and PostgreSQL, initialized through
`app:db:migrate`, including generated search columns and GIN indexes. Its
harness also verifies unique body tokens, exact UID coverage, unread state
and all 50,000 messages matching the full-text index. Both begin with empty
account databases. pl_mail's importer pages are drained synchronously without
queue waits. Neither timing includes browser rendering or account setup.
Dakia retains extracted search text without filling its evictable attachment
cache; all raw messages still pass through its existing validated fetch path.

The pl_mail runtime is pinned to v0.4.1, revision
111ad61b1b45ae5e72cc159dfd6f931b4ed72040, image digest
sha256:fb0d453584d8e86d34459538b281bbc0aecab947c0d38864d662c549fcd0c9b9.
The PHP adapter exposes the real importer in Symfony's service container;
it does not replace its IMAP, message processing or storage code.

## Run

Requires Docker, Python 3, OpenSSL and Cargo. Build before measuring:

```sh
cargo test --release -p dakia-server --lib initial_mailbox_sync_benchmark --no-run
# Copy the printed executable path for --binary below.
python3 benchmarks/initial-sync/benchmark.py setup --results /tmp/dakia-sync-results
python3 benchmarks/initial-sync/benchmark.py pl --results /tmp/dakia-sync-results
python3 benchmarks/initial-sync/benchmark.py dakia --results /tmp/dakia-sync-results \
  --binary /absolute/path/to/printed/test/executable
python3 benchmarks/initial-sync/analyze.py --results /tmp/dakia-sync-results
python3 benchmarks/initial-sync/benchmark.py stop --results /tmp/dakia-sync-results
```

Use the same count and delay for both clients. `--delay-ms 20` adds 20 ms before
each command response to expose round-trip costs; it does not simulate a
bandwidth cap. Run clients sequentially with other workloads quiet for final
measurements. Repeat with fresh fixture processes and databases. The runner
kills Dakia's whole test process group as soon as pl_mail's measured elapsed
time expires, and records STOPPED instead of a success result.

TLS remains verified by Dakia using an explicit ephemeral trust root. The PHP
fixture adapter uses the client's documented test-only certificate exception.
All published ports bind 127.0.0.1. `setup` and `stop` only remove containers
carrying the benchmark label. Database reset is restricted to the named
synthetic database. Keep generated keys and database files outside the repo.

## Optimizations under test

- Generic IMAP cold pages fetch flags and headers together. Gmail still
  discovers server-assigned identities first. Mixed pages retain header reuse.
- Historical continuations first search the nearest 1,024 UIDs. They use that
  result only when it contains more than the requested page, proving another
  page exists within that range. Sparse and final pages fall back to all
  remaining UIDs. No UID list cache or token-format change is required.
- The 200-row page, 50-row wire chunk, header-byte and operation deadline
  limits remain. Admission is finite at six turns per five seconds, with
  one job per account and two globally.

Fixtures explicitly cover cold-page rejection after partial output, mixed
cached/new identities, dense continuation, and sparse final-page fallback.

Full-text acquisition now uses bounded pages rather than one body per five
seconds. Each page selects at most 200 candidates once, stops after a 4 MiB
soft MIME budget or two seconds between messages, and uses the existing
4 MiB per-message cap. One prefetched small-body chunk or individual message may cross the soft byte boundary.
It reuses the original stale-reference, MIME extraction, quota, retry and
identity-guarded commit paths. Successful pages may refill the executor's
finite six-turn/five-second budget; failures retain a durable cooldown.

Small generic IMAP bodies are read in groups of at most 50, after one epoch
and size check per group. Each literal is capped at 65,537 bytes, and exact
advertised sizes, sections, offsets, duplicate consistency and input UID order
are validated before any group is returned. Tagged failure never returns a
partial group. Gmail and larger messages retain the original individual path.
ASCII normalization avoids Unicode normalization passes while preserving the
original NFC/lowercase behavior for non-ASCII text.

Use a release test binary for final performance measurements:

```sh
cargo test --release -p dakia-server --lib initial_mailbox_sync_benchmark --no-run
```

Unoptimized test runs are diagnostic only. pl_mail runs in its production PHP
image on arm64 Docker; Dakia's test harness runs natively on macOS arm64.

## CPU and memory

Add `--resources` to both `pl` and `dakia` commands. The default sampling
interval is 0.25 seconds. Resource JSON and raw JSONL time series are saved
beside the sync results. The monitor and fake IMAP server are excluded.

pl_mail CPU comes from cumulative Docker cgroup CPU counters for the app and
PostgreSQL containers. Combined memory is summed at each sample before taking
its peak, rather than adding peaks reached at different times. Both cgroup
working set (usage minus inactive file cache) and anonymous/shared memory are
reported, with per-container breakdowns.

Dakia CPU and kernel peak RSS come from child-process `getrusage`; RSS and
physical footprint are sampled through macOS `proc_pid_rusage`. SQLite runs
inside the app and is included. Native macOS memory and Linux cgroup memory
use different accounting, so these measurements must keep their labels and
must not be presented as identical memory metrics. Docker VM overhead is not
charged to either app. CPU percentages use one logical core as 100%.

The resource window covers the sync job lifecycle, including worker startup
and final correctness checks. It excludes the Docker fixture and PostgreSQL setup. Native SQLite setup and
temporary account cleanup occur inside the benchmark child and are included. The existing
sync stopwatch continues to exclude startup and validation, as documented above.

Repeated inbox legacy seeding checks the existing membership index before
parsing message JSON. It still runs on later turns so newly cached legacy
rows are discovered, and it never rewrites an existing membership generation.
The admission cap is six jobs per account per five-second window, with one
job per account and two globally; serial maintenance retains priority.
