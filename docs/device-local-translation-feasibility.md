# Device-local translation feasibility

Initial desktop measurements on 9 November 2026, followed by physical Android testing. Mobile translation is now verified on a Galaxy S20 FE, and the updated Firefox runtime reduced measured memory use. See the [current Firefox comparison and mobile results](device-local-translation-mobile.md). The desktop tables below preserve the original runtime baseline; they are not measurements of the updated default runtime.

## What was built

The [standalone prototype](../benchmarks/device-translation/README.md) moves Dakia's existing pinned Bergamot engine into a browser Web Worker. It translates synthetic messages from Estonian, Arabic, Chinese, Japanese and German into English. No production code, mailbox data or deployment was changed.

Today, `apps/web/src/Translation.tsx` queues server jobs and polls their state. `crates/server/src/models.rs` reads message content and invokes `workers/translation/service.mjs` on the host. The existing engine already executes locally relative to that host, but a remotely hosted Dakia installation still translates on its server. The prototype verifies moving inference to the reading device.

The browser downloads verified public language packs from the loopback server, stores weights in Cache Storage and passes text directly to the worker. A separate service worker caches only this prototype's static runtime files. Neither input text nor translated output is persisted. Sentence caching is disabled. One worker and one language model are active at a time.

## Measured performance

Hardware: Apple M1 Max, MacBookPro18,4, 64 GiB RAM. Software: installed Google Chrome 157.0.8103.39 in headless mode, macOS arm64, Node 22.23.2. Chrome reports a reduced/compatibility user agent and `deviceMemory` of 32; the host hardware query confirms 64 GiB. Asset hashes and the repository base commit are recorded in [environment.json](../benchmarks/device-translation/results/environment.json).

Each language uses one synthetic message about a meeting, invoice, deadline, amount, payment prohibition and contact address. Medium inputs repeat it five times; long inputs repeat it 25 times. These repetitions measure scaling, not linguistic diversity. There are ten warm short samples, ten medium samples and three long samples per language. Medians below are calculated from the raw sample timings.

| Source to English | Pack MB | Short median | Medium median | Long median | WASM heap MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Estonian | 37.0 | 96 ms | 431 ms | 1,782 ms | 522 |
| Arabic | 37.0 | 88 ms | 406 ms | 1,823 ms | 523 |
| Chinese | 70.1 | 87 ms | 382 ms | 1,851 ms | 848 |
| Japanese | 70.3 | 81 ms | 353 ms | 1,724 ms | 853 |
| German | 37.3 | 107 ms | 492 ms | 2,038 ms | 523 |

Short inputs contain 86 to 234 UTF-16 code units, medium 438 to 1,178, and long 2,198 to 5,898. The meaning and paragraph count are comparable across languages; character count is not. Pack MB uses decimal units; heap MiB uses binary units.

Short-message p95 values were 90 to 108 ms. With only ten samples, the nearest-rank p95 is the maximum observed sample, not a reliable population tail estimate. Long-message samples are too few for a useful tail estimate.

Fresh workers with already-saved packs returned a short translation in 287 to 492 ms across 15 trials, including cache read, hash verification, runtime initialization, model load and inference. First inference after the initial load took 163 to 314 ms, excluding setup. All five upstream packs total 251.8 MB. The separate preparation step took 56.0 seconds for sequential public downloads, integrity checks and writes on this connection. Browser pack-loading timings use loopback delivery and must not be presented as internet download timings.

During the online benchmark, the page recorded two long tasks, 60 and 61 ms, and maximum 20 ms timer drift of 67.6 ms. The offline repeat recorded no long tasks and maximum drift of 43.6 ms. Translation ran off the main thread, although pack copies, rendering and other page work can still briefly affect responsiveness.

The memory figure is the worker's WASM linear-memory allocation, not total browser RSS or measured physical RAM consumption. It excludes browser overhead, JavaScript buffers, Cache Storage and temporary copies. It is still a substantial budget to reserve. A separate probe measured Estonian at 52.8 MiB after model load and 522.2 MiB after its first short translation; Japanese rose from 91.1 to 852.7 MiB. Longer messages did not increase those allocations in that probe.

An experiment reduced the configured workspace from 128 to 16 MiB and mini-batch words from 1,024 to 256. All five short outputs matched the baseline, but heap allocations stayed identical. Timing differences came from sequential runs without controlled thermal/background conditions and do not establish an optimization. Lowering these settings alone does not resolve the memory issue.

## Behavior and quality checks

All five language pairs completed again with Chrome set offline. Every pack load was a cache hit, and the short outputs exactly matched the online outputs. A subsequent offline reload successfully loaded the page, created a new worker and translated Estonian. An uncached fetch failed, confirming the browser network was blocked. Corrupted model bytes were rejected and evicted. Terminating an in-flight translation rejected the request, and a fresh worker recovered successfully.

The HTML fixture retained its table, paragraph and exact link destination for each language, online and offline. This fixture is controlled input. It does not validate handling of hostile or malformed email HTML; production must retain sanitization before and after translation and the existing safe mail renderer.

All five short outputs preserved invoice number 4821, amount 125, the contact address and the prohibition on paying before confirmation. English output inspection found awkward wording: Estonian adds “Hey! Hi!” and German renders “bis Freitag” as “to Friday”, making the deadline ambiguous. This is useful comprehension assistance, not evidence of dependable translation for payment instructions. The references are synthetic, and there was no independent bilingual review, representative mail corpus or BLEU/COMET evaluation.

Both prototype pages were inspected in Chrome at 1280×800 light and 390×500 dark. Buttons had visible focus and no horizontal overflow; benchmark and saved-pack runs were activated through Tab/Enter. The native language selector was reached by keyboard and changed using typeahead. Tab and Shift+Tab returned through its controls. The saved `keyboard-checks.sh` browser checks passed. Public pack persistence and offline reload were exercised. Review screenshots are saved with the raw results. No production dialog, menu, settings section or mail shortcut changed.

## Alternatives

| Approach | Finding | Implication for Dakia |
| --- | --- | --- |
| Existing Bergamot WASM in a worker | Verified on this Mac in Chrome, including offline operation. Uses CPU and existing pinned weights. | Best supported prototype route; memory requires careful device qualification. |
| Chrome Translator API | API exposed; Arabic, German, Japanese and Chinese reported downloadable. Estonian reported unavailable. German pack acquisition produced no progress and was aborted after 60 seconds. | No measured native latency. Revisit as a feature-detected option; do not depend on it as the only engine. |
| Transformers.js with ONNX models | Official documentation supports browser translation using WASM, optional WebGPU and quantized models. Not benchmarked here. | A different runtime/model evaluation, not an established faster or smaller replacement. |
| Keep host translation | Existing application path remains available. Its server latency was not measured in this experiment. | Retain for devices that cannot support local inference and for existing workflows. |

Chrome's [official Translator API documentation](https://developer.chrome.com/docs/ai/translator-api) describes on-device packs, user activation for creation, desktop-only support and no Web Worker access. The observed timeout is specific to this disposable headless session; it does not establish that regular Chrome installations cannot acquire packs. [Bergamot's documentation](https://github.com/browsermt/bergamot-translator/blob/main/wasm/module/README.md) documents browser workers, HTML and interactive translation. [Transformers.js documentation](https://huggingface.co/docs/transformers.js/en/index) describes its browser backends and translation support.

## Recommended integration

Make local translation an explicit option on qualified desktop and Android devices. The physical S20 FE follow-up supersedes the initial desktop-only recommendation. Download only the chosen source pack, expose download progress and pack removal, and check storage availability before committing a large download. The pinned Dakia registry currently has 43 source languages targeting English; this prototype benchmarks five of them. It does not add other targets or Urdu.

Keep one bounded worker per active translation session. Terminate it on sign-out, account changes, cancellation and idle expiry. Tie results to account, message, content revision, source/target languages and model revision, preserving the existing protection against stale results. Keep translated mail in memory initially; public model weights can be shared within the browser origin, while translated message content needs account-scoped lifecycle handling. Browser storage eviction must produce a clear pack-unavailable state when offline.

Use the existing sanitized body and renderer. Local HTML translation must not initiate image downloads or activate message markup. Serve pinned public assets from Dakia's origin and allow WASM compilation narrowly in applicable CSPs. A separate Estonian/Japanese smoke test passed with `crossOriginIsolated === false`, so this single-worker runtime does not require changing the whole application to COOP/COEP isolation. Recheck actual deployment headers before integrating.

Retain a clearly selected host-translation path. If the user chooses local processing, do not silently send text to host inference when a pack is missing or a worker fails. Device-local inference also does not make the whole email client server-free: Dakia's existing server still handles mailbox synchronization and delivery of mail to the browser.

Before broader rollout, measure additional Android devices, real iOS devices, low-memory desktop hardware, Safari and Firefox, battery cost, sustained workloads, browser memory pressure and pack eviction. Test target devices under normal mail reading and obtain bilingual review of a varied email corpus. Viewport emulation and CPU throttling would not substitute for that evidence. The follow-up establishes operation on one physical Android phone; broader mobile and cross-browser qualification remains incomplete.

## Evidence

The [prototype README](../benchmarks/device-translation/README.md) contains reproduction commands. Raw evidence includes [online samples](../benchmarks/device-translation/results/chrome-online.json), [offline samples](../benchmarks/device-translation/results/chrome-offline.json), [offline/cancellation/integrity checks](../benchmarks/device-translation/results/browser-checks.json), [memory stages](../benchmarks/device-translation/results/memory-profile.json), [workspace experiment](../benchmarks/device-translation/results/chrome-low-workspace.json), [native API attempt](../benchmarks/device-translation/results/chrome-native-de.json) and [nonisolated smoke test](../benchmarks/device-translation/results/nonisolated.json). Run `node benchmarks/device-translation/verify-results.mjs` to check the principal recorded results. It validates the evidence structure and behavior checks, not linguistic accuracy.
