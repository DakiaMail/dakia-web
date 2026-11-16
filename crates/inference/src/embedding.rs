//! Pinned, CPU-only multilingual E5 inference. No networking or account storage.
use anyhow::{ensure, Context, Result};
use ort::{session::Session, value::Tensor};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};
pub const EMBEDDING_DIMENSIONS: usize = 384;
pub const EMBEDDING_BATCH: usize = 8;
const MAX_TOKENS: usize = 512;
const PASSAGE_TOKENS: usize = 480;
const OVERLAP_TOKENS: usize = 64;
const TEXT_WINDOW_BYTES: usize = 16 * 1024;
const PINNED_MANIFEST: &[u8] = include_bytes!("../assets/multilingual-e5-small/MANIFEST.json");
pub type Embedding = [f32; EMBEDDING_DIMENSIONS];
/// A resumable UTF-8 byte range. The next passage overlaps token evidence;
/// offsets always advance, including whitespace and tokenizer-empty content.
#[derive(Debug, Clone)]
pub struct Passage {
    pub start: usize,
    pub end: usize,
    pub next_offset: usize,
    pub text: String,
}
pub struct LocalSearchEmbedder {
    session: Session,
    tokenizer: Tokenizer,
    query_tokenizer: Tokenizer,
    token_types: bool,
    revision: String,
}
pub fn search_model_revision() -> String {
    format!("{:x}", Sha256::digest(PINNED_MANIFEST))
}
fn verified(dir: &Path) -> Result<()> {
    ensure!(
        std::fs::read(dir.join("MANIFEST.json"))? == PINNED_MANIFEST,
        "search model manifest differs from the pinned build"
    );
    let manifest: serde_json::Value = serde_json::from_slice(PINNED_MANIFEST)?;
    let mut files = manifest["files"]
        .as_object()
        .context("invalid pinned search manifest")?
        .iter()
        .collect::<Vec<_>>();
    files.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (name, expected) in files {
        let mut file = std::fs::File::open(dir.join(name))
            .with_context(|| format!("missing search model asset: {name}"))?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        ensure!(
            Some(format!("{:x}", digest.finalize()).as_str()) == expected.as_str(),
            "search model asset hash mismatch: {name}"
        );
    }
    Ok(())
}
fn tokenizer(dir: &Path) -> Result<Tokenizer> {
    let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
        .map_err(|e| anyhow::anyhow!("could not load search tokenizer: {e}"))?;
    tokenizer
        .with_truncation(None)
        .map_err(|e| anyhow::anyhow!("could not configure search tokenizer: {e}"))?;
    // E5 uses the XLM-R vocabulary, whose pad ID is 1, not the generic default 0.
    tokenizer.with_padding(Some(PaddingParams {
        pad_id: 1,
        pad_token: "<pad>".into(),
        ..Default::default()
    }));
    Ok(tokenizer)
}
fn boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
impl LocalSearchEmbedder {
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        Self::from_dir_with_threads(dir, 2)
    }
    pub fn from_dir_with_threads(dir: impl AsRef<Path>, threads: usize) -> Result<Self> {
        ensure!(
            (1..=8).contains(&threads),
            "invalid search inference thread count"
        );
        let dir = dir.as_ref();
        verified(dir)?;
        let tokenizer = tokenizer(dir)?;
        let mut query_tokenizer = tokenizer.clone();
        query_tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| anyhow::anyhow!("could not configure search query tokenizer: {e}"))?;
        let session = Session::builder()?
            .with_intra_threads(threads)?
            .with_inter_threads(1)?
            .commit_from_file(dir.join("model.onnx"))?;
        let token_types = session
            .inputs()
            .iter()
            .any(|input| input.name() == "token_type_ids");
        Ok(Self {
            session,
            tokenizer,
            query_tokenizer,
            token_types,
            revision: search_model_revision(),
        })
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn next_passage(&self, text: &str, offset: usize) -> Result<Option<Passage>> {
        next_passage(&self.tokenizer, text, offset)
    }
    pub fn embed_query(&mut self, text: &str) -> Result<Embedding> {
        ensure!(
            text.len() <= 64 * 1024,
            "search query exceeds inference turn bound"
        );
        let encodings = self
            .query_tokenizer
            .encode_batch(vec![format!("query: {text}")], true)
            .map_err(|e| anyhow::anyhow!("search query tokenization failed: {e}"))?;
        self.infer(&encodings)?
            .pop()
            .context("search query embedding missing")
    }
    /// Passages must be produced by next_passage. Never silently truncate them.
    pub fn embed_passages(&mut self, texts: &[&str]) -> Result<Vec<Embedding>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        ensure!(
            texts.len() <= EMBEDDING_BATCH
                && texts.iter().all(|text| text.len() <= TEXT_WINDOW_BYTES),
            "search passage batch exceeds inference turn bound"
        );
        // The quantized graph's dynamic scales change with batch composition.
        // Keep each passage's inference independent of scheduler grouping and
        // padding lengths, so retries produce the same persisted vector.
        let mut vectors = Vec::with_capacity(texts.len());
        for text in texts {
            let encodings = self
                .tokenizer
                .encode_batch(vec![format!("passage: {text}")], true)
                .map_err(|e| anyhow::anyhow!("search passage tokenization failed: {e}"))?;
            vectors.extend(self.infer(&encodings)?);
        }
        Ok(vectors)
    }

    fn infer(&mut self, encodings: &[tokenizers::Encoding]) -> Result<Vec<Embedding>> {
        let batch = encodings.len();
        let tokens = encodings.first().map(|v| v.len()).unwrap_or(0);
        ensure!(
            batch > 0
                && batch <= EMBEDDING_BATCH
                && tokens > 0
                && tokens <= MAX_TOKENS
                && encodings.iter().all(|v| v.len() == tokens),
            "search embedding input shape invalid"
        );
        let ids: Vec<i64> = encodings
            .iter()
            .flat_map(|v| v.get_ids().iter().copied().map(i64::from))
            .collect();
        let mask: Vec<i64> = encodings
            .iter()
            .flat_map(|v| v.get_attention_mask().iter().copied().map(i64::from))
            .collect();
        let ids = Tensor::<i64>::from_array(([batch, tokens], ids))?;
        let attention = Tensor::<i64>::from_array(([batch, tokens], mask.clone()))?;
        let outputs = if self.token_types {
            self.session.run(ort::inputs!{"input_ids"=>ids,"attention_mask"=>attention,"token_type_ids"=>Tensor::<i64>::from_array(([batch,tokens],vec![0i64;batch*tokens]))?})?
        } else {
            self.session
                .run(ort::inputs! {"input_ids"=>ids,"attention_mask"=>attention})?
        };
        let (shape, hidden) = outputs["last_hidden_state"].try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [batch as i64, tokens as i64, EMBEDDING_DIMENSIONS as i64],
            "search embedding output shape invalid"
        );
        mean_pool(hidden, &mask, batch, tokens)
    }
}
fn next_passage(tokenizer: &Tokenizer, text: &str, offset: usize) -> Result<Option<Passage>> {
    ensure!(
        offset <= text.len() && text.is_char_boundary(offset),
        "invalid search passage cursor"
    );
    if offset == text.len() {
        return Ok(None);
    }
    let window_end = boundary(text, (offset + TEXT_WINDOW_BYTES).min(text.len()));
    let encoded = tokenizer
        .encode(&text[offset..window_end], false)
        .map_err(|e| anyhow::anyhow!("search passage tokenization failed: {e}"))?;
    let positions = encoded.get_offsets();
    let (end, next) = if positions.len() > PASSAGE_TOKENS {
        (
            offset + positions[PASSAGE_TOKENS - 1].1,
            offset + positions[PASSAGE_TOKENS - OVERLAP_TOKENS].0,
        )
    } else if window_end < text.len() && positions.len() > OVERLAP_TOKENS {
        (
            window_end,
            offset + positions[positions.len() - OVERLAP_TOKENS].0,
        )
    } else {
        (window_end, window_end)
    };
    let end = boundary(text, end.min(window_end));
    let next = boundary(text, next.min(end));
    ensure!(end > offset, "search passage tokenizer made no progress");
    Ok(Some(Passage {
        start: offset,
        end,
        next_offset: if next > offset { next } else { end },
        text: text[offset..end].to_owned(),
    }))
}
fn mean_pool(hidden: &[f32], mask: &[i64], batch: usize, tokens: usize) -> Result<Vec<Embedding>> {
    ensure!(
        hidden.len() == batch * tokens * EMBEDDING_DIMENSIONS && mask.len() == batch * tokens,
        "invalid search pooling shape"
    );
    let mut result = Vec::with_capacity(batch);
    for row in 0..batch {
        let mut sums = [0f64; EMBEDDING_DIMENSIONS];
        let mut count = 0usize;
        for token in 0..tokens {
            if mask[row * tokens + token] == 0 {
                continue;
            }
            ensure!(
                mask[row * tokens + token] == 1,
                "invalid search attention mask"
            );
            count += 1;
            for (column, sum) in sums.iter_mut().enumerate() {
                *sum += f64::from(hidden[(row * tokens + token) * EMBEDDING_DIMENSIONS + column]);
            }
        }
        ensure!(count > 0, "empty search attention mask");
        for sum in &mut sums {
            *sum /= count as f64;
        }
        let norm = sums.iter().map(|v| v * v).sum::<f64>().sqrt();
        ensure!(
            norm.is_finite() && norm > 0.,
            "non-finite or zero search embedding"
        );
        let mut vector = [0f32; EMBEDDING_DIMENSIONS];
        for (value, sum) in vector.iter_mut().zip(sums) {
            *value = (sum / norm) as f32;
        }
        result.push(vector);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn assets() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/multilingual-e5-small")
    }
    #[test]
    fn pooling_masks_padding_normalizes_and_rejects_invalid_values() {
        let mut hidden = vec![0f32; 2 * EMBEDDING_DIMENSIONS];
        hidden[0] = 3.;
        hidden[1] = 4.;
        hidden[EMBEDDING_DIMENSIONS] = f32::INFINITY;
        let pooled = mean_pool(&hidden, &[1, 0], 1, 2).unwrap();
        assert!((pooled[0][0] - 0.6).abs() < 1e-6);
        assert!((pooled[0][1] - 0.8).abs() < 1e-6);
        assert!(mean_pool(&hidden, &[0, 0], 1, 2).is_err());
        assert!(mean_pool(&hidden, &[1, 1], 1, 2).is_err());
    }
    #[test]
    fn pinned_manifest_rejects_modified_assets_before_runtime_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MANIFEST.json"), b"{}").unwrap();
        assert!(verified(dir.path()).is_err());
        std::fs::write(dir.path().join("MANIFEST.json"), PINNED_MANIFEST).unwrap();
        std::fs::write(dir.path().join("config.json"), b"changed model config").unwrap();
        let error = verified(dir.path()).unwrap_err();
        assert!(error.to_string().contains("hash mismatch: config.json"));
    }
    #[test]
    #[ignore = "requires pinned public search tokenizer; run scripts/fetch-search-model.py"]
    fn chunk_cursor_covers_later_multilingual_passages_and_empty_tokens() {
        let tokenizer = tokenizer(&assets()).unwrap();
        assert!(next_passage(&tokenizer, "é", 1).is_err());
        assert!(next_passage(&tokenizer, "", 0).unwrap().is_none());
        assert!(next_passage(&tokenizer, "text", 5).is_err());
        for text in [
            format!(
                "{} 最後の契約 東京 café e\u{301} 🙂 tailneedle",
                "meeting notes русский ".repeat(3000)
            ),
            format!("{}tailneedle", " ".repeat(100000)),
        ] {
            let mut offset = 0;
            let mut previous_end = 0;
            let mut tail = false;
            let mut turns = 0;
            while let Some(passage) = next_passage(&tokenizer, &text, offset).unwrap() {
                assert!(passage.start <= previous_end);
                assert!(passage.end > offset);
                assert!(passage.next_offset > offset);
                assert!(passage.text.len() <= TEXT_WINDOW_BYTES);
                let encoded = tokenizer
                    .encode(format!("passage: {}", passage.text), true)
                    .unwrap();
                assert!(encoded.len() <= MAX_TOKENS);
                tail |= passage.text.contains("tailneedle");
                previous_end = passage.end;
                offset = passage.next_offset;
                turns += 1;
                assert!(turns < 1000);
            }
            assert!(tail);
            assert_eq!(offset, text.len());
            assert!(turns > 1);
        }
    }
    #[test]
    #[ignore = "requires pinned public E5 assets; run scripts/fetch-search-model.py"]
    fn real_cpu_multilingual_embeddings_rank_meaning_and_preserve_batches() {
        let mut model = LocalSearchEmbedder::from_dir(assets()).unwrap();
        let texts = [
            "The parcel will arrive tomorrow. Track your shipment with this delivery number.",
            "A family recipe for baking cinnamon bread.",
            "Votre colis sera livré demain. Suivez la livraison de votre commande.",
        ];
        let vectors = model.embed_passages(&texts).unwrap();
        let q = model
            .embed_query("When will my package be delivered?")
            .unwrap();
        let score = |v: &Embedding| v.iter().zip(q).map(|(a, b)| a * b).sum::<f32>();
        assert!(score(&vectors[0]) > score(&vectors[1]));
        assert!(score(&vectors[2]) > score(&vectors[1]));
        for v in &vectors {
            assert!((v.iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 1e-5);
        }
        let single = model.embed_passages(&[texts[1]]).unwrap();
        assert!(single[0]
            .iter()
            .zip(vectors[1])
            .all(|(a, b)| (*a - b).abs() < 2e-4));
        assert!(model.embed_passages(&[&"word ".repeat(1000)]).is_err());
    }
}
