# Third-party notices

Dakia Web's original source is licensed under MPL-2.0; see `LICENSE`. This does not relicense dependencies, models, fonts or vendored code.

## Reused Dakia source

Mail-provider SMTP stage handling and inference/translation assets derive from [Dakia Desktop](https://github.com/DakiaMail/dakia-desktop), licensed MPL-2.0. Preserve the original notices and full texts in `crates/inference/assets/licenses/`. The web source repository is https://github.com/DakiaMail/dakia-web; repository access is controlled by its owner.

`crates/inference/assets/DESKTOP_THIRD_PARTY_NOTICES.md` is preserved historical provenance, not a claim that the web runtime packages a macOS library or that desktop source alone constitutes web source availability. Distributors must make the corresponding covered source available to recipients as required by MPL-2.0.

## Classifier and tokenizer

The classifier is `Ippoboi/mmbert-s-email-classifier`, revision `72de7110305b5e1d98d26aa0578482a230739c0c`, declared Apache-2.0 upstream. `MANIFEST.json` records artifact digests and the fetcher retrieves the oversized ONNX file at that exact revision.

- https://huggingface.co/Ippoboi/mmbert-s-email-classifier/tree/72de7110305b5e1d98d26aa0578482a230739c0c
- Apache-2.0 full text: `crates/inference/assets/licenses/Apache-2.0.txt`

Its base model/tokenizer family is `jhu-clsp/mmBERT-small`, declared MIT. The original export does not record the exact base-model revision. The preserved MIT notice records the upstream provenance and absence of a standalone copyright notice rather than inventing one:

- `crates/inference/assets/licenses/mmBERT-small-MIT-NOTICE.txt`

## ONNX Runtime

The Rust `ort`/`ort-sys` dependencies obtain the platform runtime according to the pinned Cargo lockfile/build. ONNX Runtime is MIT-licensed. Historical ONNX Runtime 1.23.2 license and third-party notices are preserved under `crates/inference/assets/licenses/`; the desktop notice's macOS archive description does not describe this Linux packaging. Consult the actual pinned runtime distribution's notices when redistributing it.

- https://github.com/microsoft/onnxruntime

## Bergamot and translation models

`workers/translation/vendor/` contains the Bergamot Translator runtime from `@browsermt/bergamot-translator`; `upstream-package.json` preserves package provenance. Bergamot is MPL-2.0. `translator-worker.cjs` is the CommonJS packaging adaptation beside the generated JS/WASM.

- https://github.com/browsermt/bergamot-translator
- Full MPL-2.0 text: `crates/inference/assets/licenses/MPL-2.0.txt`

Public translation models are selected from Mozilla's production Translations registry, with URLs, byte sizes and hashes pinned in `workers/translation/models.json`. The historical desktop notice preserves existing model provenance; it is not a new exhaustive license audit of every language pack. Review each artifact's upstream terms before redistribution.

- https://github.com/mozilla/translations
- https://firefox.settings.services.mozilla.com/v1/buckets/main/collections/translations-models/records

## Frontend and other dependencies

`vendor/libsqlite3-sys` preserves the MIT-licensed 0.30.1 Rust wrapper and license.
Its SQLite amalgamation is the unmodified public-domain SQLite 3.54.1 release,
replacing the wrapper's bundled 3.46.0. See `vendor/libsqlite3-sys/DAKIA.md` for
source verification, supported features and the reason for the local Cargo patch.

Inter is supplied by `@fontsource/inter` under SIL Open Font License 1.1. React, TanStack, Radix, Lucide, DOMPurify, Squire, Node workers and Rust dependencies retain their package licenses. Their exact versions are recorded in the Cargo and npm lockfiles. Preserve applicable notices when distributing dependency code. This file summarizes known provenance and is not a replacement for the upstream license texts.
