# Disposable provider fixtures

Default cargo tests start loopback-only Rust capture servers with a fresh trusted test certificate in memory. They exercise TLS/mandatory STARTTLS, invalid cert rejection before auth, raw delivered MIME/Bcc/threading/attachments, RCPT abort-before-DATA, immutable restore, SMTP lost acknowledgement, Gmail identity/labels/thread, safe move, typed failures, IDLE, timeout/reconnect and Sent uncertainty. They do not connect to owner accounts.

Run from repository root:

    python3 crates/mail-provider/fixtures/run_dovecot.py

Requires Docker, OpenSSL CLI, Rust/Cargo and OpenSSL development libraries. Real-server fixture is dovecot/dovecot:2.3.21.1 pinned at sha256:547d28f9a893c93a1d2f6dc164880cd7bfa74009106dfa46b36a6a2ee636d921. It uses random loopback IMAP/IMAPS ports, fixture-only static auth and container-owned mail storage. No host mail folders or real credentials are mounted. Test certs/keys are generated in ignored target/, never packaged as source. The runner stops its container even on failure; stopped fixture containers/volumes are retained for inspection and normal Docker cleanup.

Dovecot check covers actual IMAPS/STARTTLS, all-folder discovery including unsubscribed folders, provider search for never-cached year-2000 mail outside Inbox, serialized continuation/stale token rejection, PEEK read-neutral hydration, read/star, MOVE/COPYUID, Trash, stale references, special Sent discovery and exact-ID repeat reconciliation. It does not claim live Gmail acceptance or use permanent deletion.

For an existing disposable fixture export DAKIA_FIXTURE_IMAPS_PORT, DAKIA_FIXTURE_IMAP_PORT and DAKIA_FIXTURE_CERT (PEM path), then:

    cargo test --manifest-path crates/mail-provider/Cargo.toml --test dovecot -- --ignored

Never point fixture tests at a real mailbox.

If the pinned amd64 image cannot run under local ARM emulation, use `python3 crates/mail-provider/fixtures/run_dovecot.py --ssh-host YOUR_TEST_HOST`. This starts a separate synthetic container on an already authenticated native amd64 Docker host and forwards its random loopback ports through SSH. Only generated fixture certificates and the checked-in synthetic configuration are copied. Tests still execute locally. No owner mail volume is mounted, and the container is stopped on completion or failure.
