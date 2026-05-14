//! Pluggable embedding interface.
//!
//! The controller takes `Option<Arc<dyn Embedder>>` so deployments choose
//! their own provider. v0 ships a `DeterministicEmbedder` (hash-based,
//! self-contained, used in tests) and the trait surface. Live providers
//! (OpenAI, Cohere, local-model BGE) land as adapters in follow-up commits
//! so the controller layer doesn't depend on any HTTP client.
//!
//! `EMBEDDING_DIM = 1536` is the canonical on-disk dimension — matches
//! OpenAI text-embedding-3-small at full resolution. Adapters that produce
//! a different native dimension are expected to project (Matryoshka truncate
//! or pad) before returning, so the storage schema stays single-dim.

use async_trait::async_trait;

pub const EMBEDDING_DIM: usize = 1536;

#[async_trait]
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;

    /// Embed a batch of texts. Implementations should batch on the wire —
    /// `texts.len() == 1` is the hot path, but query expansion / dream-cycle
    /// re-embedding need batches.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedderError {
    #[error("{0}")]
    Other(String),
}

/// Hash-projection embedder. **Not semantic** — it sums per-token FNV-1a
/// hashes into a fixed-dim L2-normalized vector. Two identical texts produce
/// byte-identical embeddings. Used in tests and as the default when no real
/// provider is configured (BM25 will outperform it on real queries, but the
/// RRF plumbing exercises correctly).
pub struct DeterministicEmbedder {
    dim: usize,
}

impl DeterministicEmbedder {
    pub fn new() -> Self {
        Self { dim: EMBEDDING_DIM }
    }

    pub fn with_dim(dim: usize) -> Self {
        assert!(dim > 0, "embedder dim must be positive");
        Self { dim }
    }
}

impl Default for DeterministicEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Embedder for DeterministicEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        Ok(texts.iter().map(|t| embed_text(t, self.dim)).collect())
    }
}

fn embed_text(text: &str, dim: usize) -> Vec<f32> {
    let mut out = vec![0.0_f32; dim];
    let lower = text.to_lowercase();
    for tok in lower.split(|c: char| !c.is_alphanumeric()) {
        if tok.is_empty() {
            continue;
        }
        let h = fnv1a(tok.as_bytes());
        let idx = (h as usize) % dim;
        out[idx] += 1.0;
    }
    let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut out {
            *v /= norm;
        }
    }
    out
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce4_84222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Cosine similarity for two equal-length vectors. Returns 0.0 if either is
/// the zero vector. Used by InMemoryStorage's vector search.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(feature = "embedders-http")]
pub mod openai {
    //! OpenAI text-embedding adapter.
    //!
    //! Targets `text-embedding-3-small` at the default 1536 dimensions —
    //! matches `EMBEDDING_DIM` and the pgvector column width without
    //! projection. Switch to `text-embedding-3-large` (3072 dims) by also
    //! migrating the schema; we keep it minimal in v0.
    //!
    //! Auth: reads `OPENAI_API_KEY` from env via `from_env()`. The key is
    //! held in-memory only; no on-disk persistence here — that's the
    //! `ditto-models` SubscriptionBackend's job for the long-lived ones.

    use super::{Embedder, EmbedderError, EMBEDDING_DIM};
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};

    pub const DEFAULT_MODEL: &str = "text-embedding-3-small";
    pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

    pub struct OpenAiEmbedder {
        api_key: String,
        model: String,
        base_url: String,
        http: reqwest::Client,
    }

    impl OpenAiEmbedder {
        pub fn new(api_key: impl Into<String>) -> Self {
            Self {
                api_key: api_key.into(),
                model: DEFAULT_MODEL.into(),
                base_url: DEFAULT_BASE_URL.into(),
                http: reqwest::Client::new(),
            }
        }

        pub fn from_env() -> Result<Self, EmbedderError> {
            let key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                EmbedderError::Other("OPENAI_API_KEY not set".into())
            })?;
            Ok(Self::new(key))
        }

        pub fn with_model(mut self, model: impl Into<String>) -> Self {
            self.model = model.into();
            self
        }

        pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
            self.base_url = url.into();
            self
        }
    }

    #[derive(Serialize)]
    struct EmbedRequest<'a> {
        model: &'a str,
        input: &'a [String],
    }

    #[derive(Deserialize)]
    struct EmbedResponse {
        data: Vec<EmbedItem>,
    }

    #[derive(Deserialize)]
    struct EmbedItem {
        embedding: Vec<f32>,
        #[serde(default)]
        index: usize,
    }

    #[async_trait]
    impl Embedder for OpenAiEmbedder {
        fn dim(&self) -> usize {
            // text-embedding-3-small native dim is 1536.
            EMBEDDING_DIM
        }

        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedderError> {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            let req = EmbedRequest {
                model: &self.model,
                input: texts,
            };
            let resp = self
                .http
                .post(format!("{}/embeddings", self.base_url))
                .bearer_auth(&self.api_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| EmbedderError::Other(format!("openai send: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(EmbedderError::Other(format!(
                    "openai http {status}: {body}"
                )));
            }
            let body: EmbedResponse = resp
                .json()
                .await
                .map_err(|e| EmbedderError::Other(format!("openai parse: {e}")))?;

            // OpenAI returns items keyed by `index`; sort defensively so the
            // output order matches the input order even if the server
            // reordered (it usually doesn't, but the contract doesn't promise).
            let mut indexed: Vec<EmbedItem> = body.data;
            indexed.sort_by_key(|i| i.index);
            Ok(indexed.into_iter().map(|i| i.embedding).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deterministic_embedder_is_deterministic() {
        let e = DeterministicEmbedder::new();
        let a = e.embed(&["hello world".into()]).await.unwrap();
        let b = e.embed(&["hello world".into()]).await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn distinct_text_distinct_embeddings() {
        let e = DeterministicEmbedder::new();
        let v = e
            .embed(&["alpha beta".into(), "gamma delta".into()])
            .await
            .unwrap();
        // Cosine of two random hash-projected sentences ought to be lowish.
        let s = cosine(&v[0], &v[1]);
        assert!(s < 0.9, "expected distinct embeddings, got cosine {s}");
    }

    #[tokio::test]
    async fn batch_embed_returns_in_input_order() {
        let e = DeterministicEmbedder::new();
        let v = e
            .embed(&["a".into(), "b".into(), "c".into()])
            .await
            .unwrap();
        assert_eq!(v.len(), 3);
        let v2 = e.embed(&["a".into()]).await.unwrap();
        assert_eq!(v[0], v2[0]);
    }

    #[tokio::test]
    async fn embedding_is_l2_normalized() {
        let e = DeterministicEmbedder::new();
        let v = e.embed(&["some text".into()]).await.unwrap();
        let n: f32 = v[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5, "norm was {n}");
    }

    #[test]
    fn cosine_self_is_one() {
        let v = [0.6_f32, 0.8_f32];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = [1.0_f32, 0.0_f32];
        let b = [0.0_f32, 1.0_f32];
        assert!(cosine(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_returns_zero() {
        let a = [0.0_f32, 0.0_f32];
        let b = [1.0_f32, 0.0_f32];
        assert_eq!(cosine(&a, &b), 0.0);
    }
}
