# Firefox comparison and physical Android verification

Device-local translation works on the connected Galaxy S20 FE. The updated prototype completed Estonian, Arabic, Chinese, Japanese and German into English in physical Android Chrome. Compared with our original runtime, its highest sampled increase in Chrome process PSS fell by 33–43%. This establishes a working mobile implementation on this phone, rather than a desktop-only requirement.

The prototype now defaults to the updated runtime. Dakia's production translation routing remains a separate integration task.

## Comparison with current Firefox

The comparison pins Firefox source at `d4a9923322691e221ec1afac57715016071aa506` and its public Remote Settings `translations-wasm-v2` record version `4.0`, release `v0.6.0`. The record's compressed and decompressed hashes were verified. The decompressed runtime SHA-256 is `f38ef807636a7c994afedaab7ff8ffe0d590d21897f05139f747b80fd7bbe926`. See [provenance](../benchmarks/device-translation/firefox/provenance.json), the bundled MPL license and the saved upstream reference files.

Firefox's [release history and asset selection](https://github.com/mozilla-firefox/firefox/blob/d4a9923322691e221ec1afac57715016071aa506/toolkit/components/translations/actors/TranslationsParent.sys.mjs) explicitly identify memory savings in WASM major version 3 and compressed delivery in version 4. Our original prototype used the older `@browsermt/bergamot-translator` 0.4.9 runtime and bindings. Reducing the old runtime's configured workspace had not reduced its heap allocation; replacing the runtime did.

| Area | Original prototype | Firefox implementation and applied change |
| --- | --- | --- |
| Runtime | Older bindings and WASM | Current public v0.6.0 runtime with the current language-aware model constructor and sentence segmentation support. |
| Initial linear memory | Runtime default, 16 MiB | Start at 40 MiB, matching Firefox's empirically chosen initial allocation. |
| Model handoff | Structured-cloned model buffers | Transfer ownership to the worker, avoiding a second main-thread copy. |
| Original buffers | Retained until garbage collection | Detach the original WASM and model buffers after loading them into aligned WASM memory. Detachment is checked in the running worker. |
| Idle lifetime | Explicit termination by the benchmark | Release the worker after 15 seconds with no outstanding RPC. Activity cancels and restarts that single timer. |
| Long content | Submit the whole message | A pull-based block iterator allows the caller to request visible blocks first and stop consuming on navigation or cancellation. |
| Native allocations | Cleanup on the usual return path | Response vectors, individual responses and argument vectors are released through `finally` blocks. |
| Decoder settings | Existing CPU/int8 configuration | Keep Firefox's beam 1, workspace 128 MiB, mini-batch 1,024 and disabled sentence cache/quality model. These last settings were already present, rather than new optimizations. |

The implementation follows Firefox's [worker allocation and model configuration](https://github.com/mozilla-firefox/firefox/blob/d4a9923322691e221ec1afac57715016071aa506/toolkit/components/translations/content/translations-engine.worker.js) and [buffer transfers and idle lifecycle](https://github.com/mozilla-firefox/firefox/blob/d4a9923322691e221ec1afac57715016071aa506/toolkit/components/translations/content/translations-engine.sys.mjs). The pull-based iterator is the portable counterpart of the scheduling idea in Firefox's [viewport-aware document translator](https://github.com/mozilla-firefox/firefox/blob/d4a9923322691e221ec1afac57715016071aa506/toolkit/components/translations/content/translations-document.sys.mjs). It is tested in the prototype harness; it is not yet connected to Dakia's reader viewport.

We kept exactly the same pinned language weights to isolate runtime changes. Firefox's compressed model delivery and its privileged inference-process isolation have not been ported. The Firefox runtime runs here in an ordinary Chrome Web Worker with portable WASM GEMM. These are measurements of the adapted runtime in Chrome, not timings of Firefox's built-in translator or its privileged optimizations.

## Physical test device and method

The connected device identifies as Samsung `SM-G780G`, Galaxy S20 FE, Android 13, arm64. Android exposes 5,763,308 KiB of system RAM, consistent with a 6 GB model. Installed Chrome is `156.0.8078.79`. Its reduced web user agent reports Android 10 and `navigator.deviceMemory` reports 4; ADB hardware readings are used for the actual device details.

Chrome was controlled over USB debugging. The prototype remained on a loopback origin forwarded over USB; no mailbox was connected. Model bytes came from the prepared public-weight cache. Input and output stayed in the test tab and worker.

The primary run tests both runtimes across five pairs, sequentially. Each case includes first inference, ten warm short samples, five medium samples, three long samples and the HTML fixture. Medium and long inputs repeat the short message five and 25 times. This measures scaling, not varied linguistic content. Exact timings, outputs, battery/thermal state and per-process memory samples are saved under [mobile results](../benchmarks/device-translation/results/mobile/).

Actual process memory comes from Android `dumpsys meminfo` for every Chrome process returned by CDP `SystemInfo.getProcessInfo`, including renderers and the browser/GPU processes. Each snapshot sums their PSS and records RSS and swap PSS separately. PSS proportionally accounts for shared resident pages; adding RSS would count shared mappings multiple times, so RSS totals are not used as the headline memory metric.

The figures below subtract the median of three no-worker baseline snapshots from the highest sampled Chrome PSS during the case. They include Chrome's other existing tabs and browser activity. They are measured incremental whole-Chrome costs, not isolated worker RSS, a minimum RAM requirement or an upper bound on every transient allocation. A snapshot takes roughly 0.7 seconds across processes; sampling is approximately once per second. Vanishing processes cause a fresh sample to be retried, rather than counted as zero.

## Actual memory results

| Source to English | Original additional PSS | Updated additional PSS | Reduction | Original WASM heap | Updated WASM heap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Estonian | 349 MiB | 199 MiB | 43% | 522 MiB | 196 MiB |
| Arabic | 355 MiB | 233 MiB | 34% | 523 MiB | 196 MiB |
| Chinese | 759 MiB | 470 MiB | 38% | 848 MiB | 433 MiB |
| Japanese | 780 MiB | 470 MiB | 40% | 853 MiB | 434 MiB |
| German | 354 MiB | 238 MiB | 33% | 523 MiB | 196 MiB |

For context, the updated Japanese case's whole-Chrome PSS rose from about 390 MiB to 860 MiB. The WASM heap number describes allocated addressable capacity; the PSS number measures resident physical-memory attribution. They should not be substituted for each other.

## Performance and thermal limits

These are the initial sustained A/B run's medians. They include the concurrent Android memory sampler's overhead.

| Source | Original short | Updated short | Updated medium | Updated long |
| --- | ---: | ---: | ---: | ---: |
| Estonian | 288 ms | 288 ms | 1,333 ms | 5,542 ms |
| Arabic | 269 ms | 266 ms | 1,241 ms | 5,682 ms |
| Chinese | 260 ms | 337 ms | 1,601 ms | 8,123 ms |
| Japanese | 241 ms | 396 ms | 1,943 ms | 9,435 ms |
| German | 325 ms | 394 ms | 1,962 ms | 9,463 ms |

With already-saved packs but a fresh worker, setup plus the first short translation took about 0.97–1.78 seconds in the initial updated run. The 15-second idle release therefore trades lower background memory for this restart cost. Network download time is separate.

The runtime change improves memory; it does not establish a general speed improvement. The initial run put the original engine first and the updated engine second. Android reported thermal status 0 initially and light thermal throttling, status 1, during later Chinese/Japanese/German runs. The phone was charging over USB. Differences in runtime segmentation, output and heat prevent attributing all the later slowdown to the new runtime. The data does not measure battery drain.

A reversed-order Chinese/Japanese retest also completed after the idle-timer fix. Both engines started and ended these cases at Android thermal status 1:

| Source | Updated short | Original short, tested later | Updated long | Original long, tested later |
| --- | ---: | ---: | ---: | ---: |
| Japanese | 544 ms | 392 ms | 11,822 ms | 8,458 ms |
| Chinese | 541 ms | 421 ms | 12,353 ms | 9,220 ms |

The repeated slowdown despite reversing engine order means the updated portable runtime should be treated as a memory/latency tradeoff for these CJK samples, rather than assuming heat explains everything. Equal thermal-status categories do not make CPU frequencies or temperatures identical, so this is not a controlled attribution to a particular runtime change. The raw [mobile retest](../benchmarks/device-translation/results/mobile-retest/) records per-case thermal data. All cases finished active work extending beyond the 15-second idle timeout without being interrupted.

## Behavior and lifecycle verification

All five physical-device cases preserved the HTML fixture's table structure and exact link destination. The outputs preserved invoice 4821, amount 125, the contact address and the prohibition on paying before confirmation. The updated segmenter changes some wording, particularly CJK output. These checks are a quality smoke test; independent bilingual review and representative email samples are still needed.

The five-pair desktop regression also passed with the updated engine. Offline reload, cached-pack translation, corrupt-pack rejection/eviction, worker cancellation and recovery passed using the same desktop Chrome checks as the original prototype. Physical-device follow-ups verify transferred/detached buffers, pull-based block consumption, cancellation between blocks, idle release and recovery. In the final checks, Estonian's first block arrived in 0.99 seconds after model setup, while all five blocks took 2.86 seconds; Japanese took 1.58 seconds for the first block and 3.78 seconds for all five. These include first-inference cost, rather than warm-cache latency.

The final worker remained loaded at ten seconds and released after fifteen seconds of inactivity. Sampled whole-Chrome PSS returned from roughly 619 to 388 MiB for Estonian and 853 to 385 MiB for Japanese without a forced garbage collection. The worker then reloaded saved packs and recovered. Physical Android offline reload and fresh-worker Estonian/Japanese translation also passed with an uncached fetch confirmed blocked. [Offline evidence](../benchmarks/device-translation/results/mobile-offline.json) and [actual phone light/dark captures](../benchmarks/device-translation/results/mobile-light.png) are saved; the real page viewport was 360×666 with no horizontal overflow and visible keyboard focus.

A follow-up exposed a duplicate idle-timer bug: scheduling a timer both when receiving a response and again when returning translated text left old timers alive. The fix gives the RPC completion path sole ownership of idle scheduling. The saved failed case is retained as regression evidence; successful comparisons are stored separately. The final long-running tests completed without a timer stopping active work, and the idle test confirmed that the worker remains loaded at ten seconds before it releases after fifteen seconds.

The prototype's service worker cache version was advanced for the final client. The mobile runner refreshes only this dedicated prototype origin's static cache and verifies the client version before testing. Public model packs remain cached.

## What this supports

The original desktop-only inference was too restrictive. An ordinary mobile web page can run these translations on a 6 GB Galaxy S20 FE, with short messages completing in a fraction of a second once warm in the initial run. Longer content needs progressive presentation and cancellation, and sustained work can heat and slow the phone.

The practical implementation is one active language worker, on-demand public packs, transferable buffers, demand-driven blocks and prompt release on inactivity or cancellation. Do not silently switch from a selected local mode to host inference when memory or a pack is unavailable.

This verifies one Android phone and Chrome version. It does not establish a universal RAM minimum or certify low-end Android devices, iOS Safari, Firefox's web-content execution path, battery behavior or every language pair. Those remain device/browser qualification work rather than reasons to exclude mobile categorically.

## Reproduction and evidence

See the [prototype README](../benchmarks/device-translation/README.md) for USB setup, A/B runs and lifecycle checks. Run:

```sh
node benchmarks/device-translation/verify-results.mjs
node benchmarks/device-translation/verify-mobile-results.mjs
```

The verifiers audit the saved measurements, completion and behavior checks. Raw Android PSS/RSS samples, runtime/model metadata, the separate follow-up cases, exact Firefox source provenance and the earlier desktop baseline are preserved in the repository. No result is inferred from viewport emulation or an Android simulator.
