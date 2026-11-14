# Build and publish Dakia

For running a published image, use [hosting](../docs/hosting.md) and [operations](README.md). This page is for maintainers building images and runtime bundles.

## Published images

GitHub Actions builds application source changes on `main` for Linux AMD64 and ARM64. Source images use `ghcr.io/dakiamail/dakia-web:sha-<source-commit>` and carry the source commit in the `org.opencontainers.image.revision` label. Semantic version tags publish versioned images, then a GitHub release with a digest manifest and the paired upgrade scripts after packaged-runtime checks pass. Stable releases take priority over previews for update discovery. Manual workflow runs can build a chosen source revision.

Each native image must pass packaged classifier, translation-worker, health, Gmail OAuth, and restart checks before the multi-platform image is published. The classification check verifies the deterministic policy on synthetic messages as the normal app user. The OAuth checks use synthetic credentials with networking disabled. `edge` tracks a successful `main` build; the Compose default uses an explicitly verified digest. Pushes confined to the root README, `docs/`, or the Compose pin do not trigger image builds; other documentation paths can. See the [image workflow](../.github/workflows/images.yml) for the exact triggers and checks.

Use the source-revision label and immutable digest to identify an image. Keep operator credentials and mail data in runtime configuration and persistent volumes, outside the image.

## Build

Install the requirements and run the checks in the root README. To build the legacy runtime bundle:

```sh
sh deploy/build-bundle.sh
```

The builder installs locked npm dependencies, builds/tests the UI, tests the workers and Rust workspace, builds release binaries, and creates `deploy/bundle/runtime.tar.gz`. It packages copied binaries, web assets, workers and classification policy, excluding account data and operator secrets. Build on Linux x86_64 using Node 24 LTS and the pinned Rust toolchain. Classification requires no model files or runtime downloads.

Extract the archive into a new build directory and build the included Dockerfile with an explicit local tag. It expects `bin/`, `web/`, `workers/` beside it. The image runs as UID/GID 1000. Mount persistent storage at `/data`, set the required environment and mount private configuration files at their configured paths. Configure `DAKIA_WEB_ROOT=/app/web`; the image already sets the worker and translation model-cache paths. A private VAPID file is needed for push, not embedded in the image.

Use a read-only root filesystem, no new privileges, dropped capabilities and suitable resource limits. Networking must permit OIDC discovery/backchannel, mail provider TLS, selected model downloads and supported push endpoints. Do not bypass TLS verification to resolve routing failures. Ensure `/sw.js` serves JavaScript rather than the SPA fallback.

### Native backend release builder

Use `Dockerfile.build` when compiling Linux binaries in a container. It combines
Rust 1.98 with the Ubuntu 24.04 C runtime used by the production image. Building
against Debian Bookworm's older glibc can fail on `__isoc23_strtol` symbols from
native dependencies. Do not change runtime security or inference
features to work around a mismatched build environment.

Build the image using only its Dockerfile as context:

```sh
docker build -t dakia-release-builder - < deploy/Dockerfile.build
docker run --rm -v "$PWD:/source" -w /source dakia-release-builder \
  cargo build --release --locked -p dakia-server
```

Run this on a native Linux amd64 host. Mount a source checkout and optional
isolated Cargo/target caches only. Never mount production data or credentials
into the builder. Package all three binaries without classifier model or runtime assets,
and run the release gate before activation. Back up the stopped app before
changing the production image, retain its previous tag, and verify database
integrity, mounts, health and unchanged account/submission counts afterwards.

## Packaged runtime checks

Worker updates must include locked production dependencies. A healthy Rust server doesn't prove a Node worker can start. Against the chosen image, with no application data mounts:

```sh
sudo docker run --rm --network none --read-only \
  --entrypoint /app/dakia-classifier-check YOUR_IMAGE
sudo docker run --rm --network none \
  --entrypoint node YOUR_IMAGE /app/workers/translation/smoke.mjs
```

The first verifies deterministic classification. The second loads the HTML parser, sanitizer, and WASM engine; the supplied Dockerfile also runs this worker smoke during its build. For real host-translation acceptance, run the worker smoke with `--models` and `DAKIA_MODEL_CACHE` pointing at an isolated cache of verified public Estonian models. Never mount owner mail or credentials into a build or smoke test.
