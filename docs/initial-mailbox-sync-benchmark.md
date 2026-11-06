# Initial full mailbox sync and resource usage

Dakia meets the target against the fastest measured pl_mail baseline:
**316.82 s versus 401.73 s**, or
**126.8% throughput**, exceeding 120%. Every one of the
50,000 message bodies was downloaded, matched to its UID and indexed.

## Combined sync worker and database usage

| Measurement | pl_mail + PostgreSQL | Dakia + embedded SQLite |
|---|---:|---:|
| Full sync and indexing | 401.73 s | 316.82 s |
| Messages/second | 124.46 | 157.82 |
| Total CPU time, core-seconds | 319.86 | 302.42 |
| Average CPU, one core = 100% | 79.6% | 95.1% |
| Peak sampled CPU, one core = 100% | 182.2% | 101.2% |
| Average memory, MiB* | 193.0 working set | 33.4 RSS |
| Peak memory, MiB* | 286.5 working set | 42.4 RSS |
| IMAP commands | 3,003 | 4,521 |
| Received IMAP application bytes | 253,092,520 | 129,311,828 |
| Connections | 1 | 1 |
| Body reads | 50,000 | 50,000 |

Dakia finished 21.1% sooner and consumed 5.5% less total CPU
time in this measurement. Its average CPU utilization was higher because it
kept a core busier and finished earlier. CPU time measures total work; average
utilization measures how busy the CPU was during the run.

*Memory uses different OS accounting and is not an exact like-for-like ratio.
pl_mail runs in Linux Docker; the working set is cgroup usage minus inactive
file cache, summed across app and database at each sample. Dakia runs natively
on macOS; RSS includes its embedded SQLite workers and caches but excludes
OS file cache outside the process. No memory-reduction percentage is claimed.
The Docker VM, fake IMAP server, browser, resource sampler, enrichment and
classification workers are excluded. These figures describe the sync backend
and database, not an entire production web deployment.

### pl_mail breakdown

| Component | CPU core-seconds | Average working set, MiB | Individual peak working set, MiB |
|---|---:|---:|---:|
| App worker | 147.31 | 62.2 | 97.1 |
| PostgreSQL | 172.55 | 130.8 | 224.5 |

The combined peak is taken from simultaneous sampled sums, not the sum of
component peaks reached at different times. PostgreSQL uses its default
128 MiB shared buffers and 4 MiB work_mem. Combined peak anonymous/shared
memory was 281.6 MiB; peak total cgroup
memory including file cache was 539.2 MiB.

Dakia's sampled physical footprint averaged
16.0 MiB and peaked at
24.5 MiB. Its sampled RSS peak was
42.3 MiB; the kernel-reported lifetime RSS peak in
the comparison table also captures allocations between samples.

## What changed

- Body acquisition now advances bounded pages instead of one body per five
  seconds. Generic IMAP bodies up to 64 KiB use wire batches of at most 50.
  Epoch, size, section, offset, duplicate consistency and UID order are checked;
  tagged failures never return a partially successful batch. Gmail and larger
  bodies retain the existing individual reference checks.
- Body indexing commits once per admitted page, retaining source, membership
  and per-row quota guards. A page has at most 200 candidates, a 4 MiB soft
  MIME budget and a two-second yield check between reads. One bounded wire
  batch or individually capped message can cross the soft byte boundary.
- Admission is six turns per account per five seconds, with one active job
  per account and two globally. Serial maintenance keeps priority; failures
  retain durable cooldowns. This removes avoidable idle time without raising
  per-page memory limits or global concurrency.
- Historical continuation searches use a 1,024-UID window, with a full
  remaining-range fallback for sparse/final pages. Cold generic IMAP pages
  avoid redundant flag-only FETCH commands. ASCII text avoids unnecessary
  Unicode normalization passes while non-ASCII behavior is retained.
- Inbox legacy seeding checks the membership index before decoding JSON for
  already-seeded rows. It still discovers later cached rows and preserves
  existing generations. A focused 5,000-row/50-pass probe fell from
  248 ms to 86 ms.

Dakia still makes more IMAP commands than pl_mail because metadata discovery
and size-verified body acquisition are separate. It receives substantially
fewer bytes because it avoids repeated enumeration of the entire remaining
UID set. Higher-latency networks and different message sizes can change the
comparison.

## Measurement and correctness

The deterministic fixture has one INBOX with 50,000 distinct ASCII plain-text
messages, 1,956 to 1,974 bytes each, totaling 98,645,576 raw bytes. TLS loopback
has no injected delay. Both clients verified exact UID coverage 1 through
50,000, unique body tokens, unread state and all 50,000 entries matching their
full-text index. Dakia additionally verified folder memberships and no dirty
or failed body-index rows. No mutation or rejected command was observed.

pl_mail v0.4.1 uses its real ImapMailboxImporter, PostgreSQL 18 and normal
production migrations, including the generated search vector and GIN indexes.
Its importer pages are drained without queue waits. Dakia uses the real
AccountProvider, MailRuntime, production executor and SQLite in a release test
build. Earlier pl_mail timings were 445.42 and 469.67 seconds; the target is
verified against the faster 401.73-second resource-profiled run.

Sync timing includes scheduling, acquisition, parsing, persistence and indexing.
Resource profiling covers the worker job lifecycle, including initialization,
validation and cleanup. Native SQLite setup occurs inside that child process;
Docker/PostgreSQL migrations and fixture setup occur before resource sampling.
These small window differences are recorded rather than silently treated as
identical. Resource lifecycle windows were 401.96 s for pl_mail
and 317.96 s for Dakia.

CPU uses cumulative kernel counters, with 100% meaning one logical core.
Docker app and database cgroups are combined; native child rusage includes all
SQLite threads. Memory and interval CPU peaks use nominal 250 ms samples
(1551 pl_mail, 1256 Dakia samples). Darwin CPU clock conversion
was checked against kernel process CPU time, and sampled totals agree with
final child rusage. Both samplers completed without errors. The fake server
and sampler's own CPU/memory are excluded. macOS versus Linux Docker and the
small synthetic corpus limit generalization to production installations.

271 release regression tests passed: 126 server, 99 provider and 46 storage.
The full 50,000-body benchmark and resource-enabled process-group cutoff test
passed separately. Slower diagnostic runs were stopped early. No UI behavior
changed.

## Reproduce the benchmark

Requirements: Docker, Cargo, Python 3, OpenSSL and macOS for native memory sampling.
Build before timing, and use the printed release test executable path below.

```sh
cargo test --release -p dakia-server --lib initial_mailbox_sync_benchmark --no-run
python3 benchmarks/initial-sync/benchmark.py setup --results /tmp/dakia-sync-results
python3 benchmarks/initial-sync/benchmark.py pl --results /tmp/dakia-sync-results --resources
python3 benchmarks/initial-sync/benchmark.py dakia --results /tmp/dakia-sync-results \
  --resources --binary /absolute/path/to/printed/release/test/executable
python3 benchmarks/initial-sync/analyze.py --results /tmp/dakia-sync-results
python3 benchmarks/initial-sync/benchmark.py stop --results /tmp/dakia-sync-results
```

Use the same message count and injected delay for both clients. The default is
50,000 messages and zero delay. Run clients sequentially without builds or
other benchmark jobs running. The runner kills the entire Dakia process group
at pl_mail's measured duration, including when resource sampling is enabled.
The fixture binds only loopback ports and uses generated test TLS certificates.
Do not use owner mail, production database mounts or real credentials.

The scripts initialize pl_mail with its normal production migrations and drain
its real importer synchronously. Dakia's ignored test injects a trusted fixture
certificate into its real AccountProvider and runs the production executor.
Correctness assertions and `analyze.py` reject metadata-only completion, missing
body tokens, incomplete indexing, rejected commands and failed resource capture.
The archived fixture is 50,000 small plain-text messages, not a varied production
mailbox; larger MIME, attachments and network latency need separate measurements.

## Durable evidence

- [Exact results and source/binary hashes](../benchmarks/initial-sync/results.json).
- [Captured run evidence](../benchmarks/initial-sync/evidence/2026-11-06/), including
  full-body completion logs, IMAP counters, component CPU/memory summaries,
  compressed raw time series and regression logs.
- [Harness details](../benchmarks/initial-sync/README.md).

Generated certificates, private keys, runtime database files and binaries are
not stored with the report. The manifest records the benchmark source hashes;
the recorded macOS binary hash identifies the measured executable and is not
an assertion about a later Linux deployment artifact.

## Deployment verification

The changes were [deployed and functionally verified](initial-sync-deployment-2026-11-06.md) on Linux amd64 after the benchmark.
