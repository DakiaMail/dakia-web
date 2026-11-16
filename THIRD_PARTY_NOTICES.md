# Third-party notices

Dakia Web's original source is licensed under MPL-2.0; see `LICENSE`. This does not relicense dependencies, models, fonts or vendored code.

## Reused Dakia source

Mail-provider SMTP stage handling and translation assets derive from [Dakia Desktop](https://github.com/DakiaMail/dakia-desktop), licensed MPL-2.0. See `LICENSE` for the full license. The original desktop notices remain in `workers/translation/DESKTOP_THIRD_PARTY_NOTICES.md` as historical provenance; removed desktop classifier/runtime descriptions do not describe this distribution.

## Bergamot and translation models

`workers/translation/vendor/` contains the Bergamot Translator runtime from `@browsermt/bergamot-translator`; `upstream-package.json` preserves package provenance. Bergamot is MPL-2.0. `translator-worker.cjs` is the CommonJS packaging adaptation beside the generated JS/WASM.

- https://github.com/browsermt/bergamot-translator
- Full MPL-2.0 text: `LICENSE`

Public translation models are selected from Mozilla's production Translations registry, with URLs, byte sizes and hashes pinned in `workers/translation/models.json`. The historical desktop notice preserves existing model provenance; it is not a new exhaustive license audit of every language pack. Review each artifact's upstream terms before redistribution.

- https://github.com/mozilla/translations
- https://firefox.settings.services.mozilla.com/v1/buckets/main/collections/translations-models/records

## Frontend and other dependencies

`vendor/libsqlite3-sys` preserves the MIT-licensed 0.30.1 Rust wrapper and license.
Its SQLite amalgamation is the unmodified public-domain SQLite 3.54.1 release,
replacing the wrapper's bundled 3.46.0. See `vendor/libsqlite3-sys/DAKIA.md` for
source verification, supported features and the reason for the local Cargo patch.

Inter is supplied by `@fontsource/inter` under SIL Open Font License 1.1. React, TanStack, Radix, Mantine (MIT), Lucide, DOMPurify, Squire, Node workers and Rust dependencies retain their package licenses. Their exact versions are recorded in the Cargo and npm lockfiles. Preserve applicable notices when distributing dependency code. This file summarizes known provenance and is not a replacement for the upstream license texts.

## Browser translation

Dakia includes the Mozilla Bergamot v0.6.0 WASM runtime under MPL-2.0. Its JavaScript loader has an ES module export and an explicit `globalThis` reference for module compatibility. Pinned upstream source, artifact hashes and the complete license are in `workers/translation/browser/provenance.json` and `workers/translation/browser/LICENSE`. Source: https://github.com/mozilla-firefox/firefox/tree/d4a9923322691e221ec1afac57715016071aa506/toolkit/components/translations.

## Attachment text extraction

Attachment search uses Firecrawl `anydoc` 0.2.4 and its `pdf-inspector` dependency,
locked in `Cargo.lock`. Both declare MIT licenses. Their full notices are
preserved in `crates/inference/assets/licenses/anydoc-MIT.txt` and
`crates/inference/assets/licenses/pdf-inspector-MIT.txt`, included by the existing
license packaging step. Conversion runs locally without OCR or hosted APIs.

- https://github.com/firecrawl/anydoc
- https://github.com/firecrawl/pdf-inspector

## Multilingual search embeddings

Local search uses `intfloat/multilingual-e5-small`, upstream revision
`614241f622f53c4eeff9890bdc4f31cfecc418b3`, declared MIT. Its quantized ONNX
conversion is from `Xenova/multilingual-e5-small`, revision
`761b726dd34fb83930e26aab4e9ac3899aa1fa78`. The pinned manifest records SHA-256
hashes for the ONNX model, tokenizer and configuration. The runtime verifies the
manifest against the build and verifies each asset before loading it.

- https://huggingface.co/intfloat/multilingual-e5-small
- https://huggingface.co/Xenova/multilingual-e5-small/tree/761b726dd34fb83930e26aab4e9ac3899aa1fa78
- Preserved upstream E5 MIT license: `crates/inference/assets/licenses/multilingual-e5-MIT.txt`
- E5 upstream source and license: https://github.com/microsoft/unilm/tree/master/e5

## Local vector acceleration

SQLite search uses USearch 2.26.6 and NumKong 7.8.7, locked in `Cargo.lock`,
under Apache License 2.0. The preserved upstream notices are included in the
existing license packaging step:

- `crates/inference/assets/licenses/USearch-Apache-2.0.txt`, from https://github.com/unum-cloud/USearch/tree/a985f77553d2d58d5ef1516eafaae81480bb3c30
- `crates/inference/assets/licenses/NumKong-Apache-2.0.txt`, from https://github.com/ashvardanian/NumKong/tree/8e591120fb2072791f519ef7268e9cab843eb40f

Canonical search tokenization uses unicode-general-category 1.1.0 (Apache-2.0),
locked in `Cargo.lock`. Its license is included at
`crates/inference/assets/licenses/unicode-general-category-Apache-2.0.txt`.


The optional search repair operator image includes PostgreSQL client tools under
the PostgreSQL License, SQLite 3.54.1 public-domain sources, Psycopg 3.3.8 under
LGPL-3.0-only, the corresponding psycopg-binary distribution,
and typing-extensions 4.16.0 under the Python Software Foundation License. Package
license files remain in the operator image's PostgreSQL installation and Python
virtual environment. SQLite repair sources are downloaded only from the pinned
upstream archive and verified by SHA-256 before compilation.
