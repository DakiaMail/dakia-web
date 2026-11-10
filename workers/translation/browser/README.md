# Browser translation runtime

Bergamot v0.6.0, Firefox Remote Settings runtime major version 4. The binary is the verified decompression of the pinned Mozilla artifact in `provenance.json`. The upstream loader is pinned to the listed Firefox source revision, with an ES module export appended and the classic-script global `this` reference changed to `globalThis` for strict-mode compatibility. Preserve the MPL license in this directory.

The app's module worker imports this loader. Vite bundles the worker as a static asset and emits the WASM separately. WASM is excluded from app-shell prefetch; the translation client verifies and explicitly caches it when local translation is requested. Model weights are the existing pinned `../models.json` registry, fetched through an authenticated, hash-allowlisted server endpoint. No message text enters the public-weight cache.

The worker uses Firefox's 40 MiB initial allocation, decoder configuration, aligned model memory, zero sentence cache and input-buffer detachment. Its native vectors and response handles are released after translation. The reader owns an abortable operation and the worker is released on navigation, sign-out, cancellation or fifteen seconds of inactivity.
