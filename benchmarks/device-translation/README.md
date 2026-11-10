# Device-local translation prototype

This standalone experiment defaults to Firefox's current public Bergamot v0.6.0 runtime in a Chrome Web Worker. Add `?engine=legacy` to compare Dakia's older pinned runtime. It uses synthetic messages and never connects to a mailbox. Production code and translation routing are unchanged.

The [Firefox comparison and physical Android report](../../docs/device-local-translation-mobile.md) contains the current findings. The [initial feasibility report](../../docs/device-local-translation-feasibility.md) explains the measurements and recommendation. Raw results, environment details and review screenshots are in [results/](results/).

## Run

Requires Node 22+ and Google Chrome. No npm install is needed.

```sh
node benchmarks/device-translation/server.mjs --prepare
node benchmarks/device-translation/server.mjs
```

Open `http://127.0.0.1:4178` in Chrome. The preparation step downloads 251,751,668 bytes of pinned public Mozilla weights, verifies their sizes and SHA-256 digests, and saves them under the system temporary directory at `dakia-device-translation-models`. Set `DAKIA_PROTOTYPE_CACHE` to use another cache directory. Downloads are sequential; the browser benchmark's pack-loading time does not include these upstream downloads.

Use **Run benchmark** once, then take Chrome offline and use **Test saved packs**. Each run takes roughly a minute. Do not overlap runs or diagnostics. Both buttons benchmark five language pairs, short/medium/long inputs, HTML and three fresh-worker starts per pair. Use **Save results** before reloading: results and synthetic translations are held in memory. Only public runtime files and public model weights persist in Cache Storage.

`http://127.0.0.1:4178/native.html` tests Chrome's built-in Translator API. Choose a language and activate **Run benchmark**. This separate experiment limits language-pack acquisition to 60 seconds. The measured German attempt timed out; no native latency result is claimed.

## Reproduce Chrome checks with agent-browser

Use the installed Google Chrome executable, rather than downloading another browser:

```sh
agent-browser --session dakia-translation --executable-path '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' open http://127.0.0.1:4178
agent-browser --session dakia-translation wait --fn 'window.prototypeReady === true'
agent-browser --session dakia-translation press Tab
agent-browser --session dakia-translation press Enter
agent-browser --session dakia-translation wait --fn 'window.results?.complete === true || !!window.results?.error'
agent-browser --session dakia-translation eval 'window.results' > /tmp/chrome-online.json
```

The CLI's default wait timeout can be shorter than a benchmark. After a timeout, inspect `window.results`; the benchmark continues in the page. Wait again without starting another run.

```sh
agent-browser --session dakia-translation set offline on
agent-browser --session dakia-translation press Tab
agent-browser --session dakia-translation press Enter
agent-browser --session dakia-translation wait --fn 'window.results?.complete === true || !!window.results?.error'
agent-browser --session dakia-translation eval 'window.results' > /tmp/chrome-offline.json
agent-browser --session dakia-translation reload
agent-browser --session dakia-translation wait --fn 'window.prototypeReady === true'
agent-browser --session dakia-translation eval --stdin < benchmarks/device-translation/browser-checks.js
agent-browser --session dakia-translation set offline off
```

The diagnostic checks an actual blocked fetch, offline page reload, offline translation with a fresh worker, corrupt-pack rejection and eviction, explicit worker cancellation and recovery. It restores the original public model after corrupting its disposable cache entry.

For the workspace experiment, with the original page selected and the network online:

```sh
agent-browser --session dakia-translation eval --stdin < benchmarks/device-translation/tuning.js
agent-browser --session dakia-translation eval 'window.tuningResults'
```

The tuning script starts asynchronously. Poll until `complete` or `error`. It tests a derived worker with `workspace: 16` and `mini-batch-words: 256`, compared with the pinned runtime's 128 and 1024. The server changes only this prototype response; it does not modify vendored files. Heap allocation did not improve.

To check the absence of cross-origin isolation requirements:

```sh
PORT=4179 DAKIA_PROTOTYPE_ISOLATION=0 node benchmarks/device-translation/server.mjs
```

Open that origin and run the same prototype. Estonian and Japanese short-message translation passed with `crossOriginIsolated === false`.

Validate the recorded evidence:

```sh
node benchmarks/device-translation/verify-results.mjs
node benchmarks/device-translation/verify-mobile-results.mjs
sh benchmarks/device-translation/keyboard-checks.sh
```

The evidence verifier validates result completeness, samples, literal preservation, HTML, matching offline output, cache integrity and cancellation checks. The browser script tests Tab/Shift+Tab order, visible focus and native selector typeahead. It does not certify translation quality.

## Scope and cleanup

The server binds only to `127.0.0.1` and serves an explicit asset allowlist. It accepts GET requests only. The CSP permits same-origin assets and WASM compilation. Models are served from the prepared public-weight cache; text is passed directly to the worker, never to the loopback server.

The service worker is scoped to this standalone origin. It is intentionally separate from Dakia's production service worker. To refresh prototype source after editing, unregister its service worker and clear its two named caches in Chrome DevTools: `dakia-translation-prototype-static-v5` and `dakia-translation-prototype-models-v1`. Closing the tab destroys workers; clearing site data removes saved packs. Stop the server with Ctrl+C.

Preserve Bergamot's MPL notices and model provenance when reusing these assets. See [Dakia's third-party notices](../../crates/inference/assets/DESKTOP_THIRD_PARTY_NOTICES.md).


## Physical Android benchmarks

The tested device is a Galaxy S20 FE (`SM-G780G`), Android 13, approximately 6 GB RAM, with Chrome 156. The test uses real ARM hardware, not viewport or CPU emulation. Install Android platform-tools and authorize USB debugging on the phone first. Use a dedicated loopback port:

```sh
PORT=4197 node benchmarks/device-translation/server.mjs
```

In another terminal:

```sh
adb reverse tcp:4197 tcp:4197
adb forward tcp:9223 localabstract:chrome_devtools_remote
adb shell am start -a android.intent.action.VIEW -d http://127.0.0.1:4197 -p com.android.chrome
agent-browser --session dakia-mobile --cdp 9223 tab new http://127.0.0.1:4197
node benchmarks/device-translation/mobile-benchmark.mjs
```

The CLI navigates only its disposable prototype tab. It runs both engines and five pairs while sampling Android PSS/RSS for the Chrome processes reported by CDP. It keeps public model packs, refreshes this prototype's static cache before each case, and verifies the client version. Other Chrome tabs remain open, so the report uses whole-Chrome baseline-relative PSS rather than claiming isolated worker memory. Sampling itself adds overhead to latency measurements. Each case records thermal/battery state; sequential runs can heat the phone.

Optional controls: `DAKIA_MOBILE_URL`, `DAKIA_MOBILE_SESSION`, `DAKIA_MOBILE_ENGINES=firefox,legacy`, `DAKIA_MOBILE_SOURCES=ja,zh`, and `DAKIA_MOBILE_RETEST=1` for a separate result directory. `DAKIA_MOBILE_COOLDOWN=1` waits for thermal status 0 before each case, with a four-minute limit. The recorded reversed-order run does not force cooldown and retains its actual thermal readings.

For the final lifecycle follow-up, first run or load Estonian and Japanese so their packs are saved on that origin, then:

```sh
DAKIA_MOBILE_FOLLOWUP=1 DAKIA_MOBILE_ENGINES=firefox DAKIA_MOBILE_SOURCES=et,ja node benchmarks/device-translation/mobile-benchmark.mjs
```

This checks actual transferred/detached buffers, first-block presentation, cancellation between blocks, an idle worker still present at ten seconds, automatic release after fifteen seconds, physical-memory return toward baseline and fresh-worker recovery. The separate `optimized-checks.js` performs the same client checks in a prepared desktop tab.

The prototype's pull-based `translateBlocks()` yields independently valid text/HTML blocks only when requested. A reader can stop consuming when a block is no longer visible. Automatic integration with Dakia's reader viewport is not included in this prototype.

When finished, close only the disposable test tabs and remove the forwards you created:

```sh
adb reverse --remove tcp:4197
adb forward --remove tcp:9223
```

The updated runtime, its MPL license and pinned provenance are under `firefox/`. The model weights are unchanged, allowing the runtime comparison to use the same input models. Production translation workers and routes are not modified by this experiment.
