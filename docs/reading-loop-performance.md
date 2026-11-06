# Reading-loop performance

Phase one builds on the merged initial-sync work in PR #72. It changes foreground
reading without changing mailbox synchronization or persisting mail on the device.

A 150 ms focus or pointer dwell warms one message body. Opening a message warms
one following row in the same order used by keyboard navigation. There is one
speculative request at a time and one replaceable pending intent. Foreground body
loads take priority; opening the same message shares the existing request. Scope
changes, lost focus, hidden tabs and going offline cancel unused speculative work.
An open reader retains ownership of its request. Save-Data and 2G connections skip
prefetching. Failures do not retry speculatively. At most three unused speculative
bodies are retained, with a 60-second inactive cache lifetime. Message bodies keep
the existing server size limits and 30-second request timeout.

Prefetching uses only the normal authorized body endpoint. It never mounts the
reader, requests remote images, marks mail read, discovers conversations, or
submits actions. Mail stays in the existing in-memory account-scoped query cache.

The renderer previously parsed the initial email document a second time in its
iframe after load, replacing its body and styles with identical content. Initial
load now retains the original document. Later body, image and plain-text theme
updates still use the existing sanitized update channel and sandbox.

## Reproduction

Use Node 24 and Google Chrome:

```
npm ci
npm test
npm run build
npm exec --workspace @dakia/web playwright -- test --config playwright.reading.config.ts
DAKIA_READING_FULL=1 npm exec --workspace @dakia/web playwright -- test --config playwright.reading.config.ts
```

The reading-loop fixture measures open, reply, return, next and archive using
keyboard actions with disposable mail. Bodies have a controlled 180 ms response
delay and each focused row has the same 450 ms dwell in baseline and candidate.
JSON attachments include timings, body requests and main-frame long tasks. The
measurements describe this controlled fixture, not live-provider latency or a
production percentile guarantee. Cold opens without dwell still use the normal
foreground loading path. Other-account bodies cannot satisfy the selected key.

## Controlled comparison

On merged main `af5f801`, focused opens measured 224–240 ms (mean 229 ms).
The candidate measured 34–40 ms (mean 36 ms), with the same four body requests.
Reply measured 20–43 ms, return 10–34 ms, cached next 30 ms and optimistic archive
20 ms. Neither run recorded main-frame long tasks in this small-message fixture.
This demonstrates removal of the body network wait after intentional dwell;
it does not establish that large real messages have no rendering stalls.
Raw samples are in `docs/benchmarks/reading-loop/`.

The full-suite runner uses an isolated preview port. Email-security route fixtures
accept that configured origin, keeping their existing external-request checks.
