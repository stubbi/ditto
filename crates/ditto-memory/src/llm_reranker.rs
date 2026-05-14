//! LLM-as-reranker.
//!
//! Why use a chat-completions LLM rather than a dedicated rerank API:
//! - Operator already pays for one OpenRouter (or OpenAI) key — adding
//!   Cohere/Voyage/Jina widens the auth surface for no quality win.
//! - A capable instruction-tuned model is competitive with single-vector
//!   cosine on out-of-domain queries; the gap to a dedicated late-
//!   interaction model (ColBERTv2, JaColBERT) is real but not the
//!   first-order miss on conversational data.
//!
//! Single round-trip: the LLM sees the query plus a numbered list of
//! candidates and returns the indices in ranked order. One call per
//! search, not one per candidate (which would scale O(k) calls).
//! Late-interaction kernels live in `reranker.rs` as a future swap-in.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::reranker::Reranker;
use crate::search::SearchResult;

#[derive(Debug, thiserror::Error)]
pub enum LlmRerankerError {
    #[error("env var missing: {0}")]
    EnvMissing(&'static str),
    #[cfg(feature = "embedders-http")]
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("decode: {0}")]
    Decode(String),
}

/// Reranker that asks an OpenAI-compatible chat model to re-order
/// candidates. Cheap to clone — internal `reqwest::Client` is Arc'd.
#[cfg(feature = "embedders-http")]
#[derive(Clone)]
pub struct LlmReranker {
    api_key: String,
    base_url: String,
    model: String,
    client: reqwest::Client,
}

#[cfg(feature = "embedders-http")]
impl LlmReranker {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "openai/gpt-4o-mini".into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest builder"),
        }
    }

    pub fn from_env_openrouter() -> Result<Self, LlmRerankerError> {
        let key = std::env::var("OPENROUTER_API_KEY")
            .map_err(|_| LlmRerankerError::EnvMissing("OPENROUTER_API_KEY"))?;
        Ok(Self::new(key))
    }

    pub fn with_model(mut self, m: impl Into<String>) -> Self {
        self.model = m.into();
        self
    }

    pub fn with_base_url(mut self, u: impl Into<String>) -> Self {
        self.base_url = u.into();
        self
    }

    async fn call(&self, query: &str, candidates: &[SearchResult]) -> Result<Vec<usize>, LlmRerankerError> {
        let mut listing = String::new();
        for (i, c) in candidates.iter().enumerate() {
            // Truncate long contents — most reranking signal sits in the
            // first ~200 chars and we don't want to pay for a huge prompt.
            let snip = if c.content.len() > 300 {
                format!("{}…", &c.content[..300])
            } else {
                c.content.clone()
            };
            listing.push_str(&format!("[{i}] {snip}\n"));
        }
        let user = format!(
            "Query: {query}\n\n\
             Candidates (most-to-least relevant should appear first in your output):\n\
             {listing}\n\
             Return JSON: {{\"ranking\": [<integer indices in order>]}}. Include EVERY index. Be conservative — if uncertain, preserve the input order for ties."
        );
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": user},
            ],
            "temperature": 0.0,
            "max_tokens": 256,
            "response_format": {"type": "json_object"},
        });
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        let value: Value = resp.json().await?;
        let content = value
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| LlmRerankerError::Decode("missing message content".into()))?;
        let parsed: RawRanking = serde_json::from_str(content)
            .map_err(|e| LlmRerankerError::Decode(format!("parse: {e}; raw={content}")))?;
        Ok(parsed.ranking)
    }
}

const SYSTEM_PROMPT: &str = "You re-rank retrieved memory snippets for an \
agent's question-answering system. Given a query and numbered candidates, \
return ALL candidate indices in order from most relevant to least \
relevant to the query. Relevance means: does the snippet contain (or \
strongly support) the information needed to answer the query? Surface- \
level keyword overlap is a weak signal; prefer snippets that contain the \
specific entities, facts, dates, or relationships the query asks about. \
Output JSON only: {\"ranking\": [<indices>]}. Include every index exactly \
once.";

#[derive(Debug, Deserialize)]
struct RawRanking {
    ranking: Vec<usize>,
}

#[cfg(feature = "embedders-http")]
#[async_trait]
impl Reranker for LlmReranker {
    async fn rerank(&self, query: &str, results: Vec<SearchResult>) -> Vec<SearchResult> {
        if results.len() <= 1 {
            return results;
        }
        match self.call(query, &results).await {
            Ok(order) => {
                // Permissive validator. The LLM commonly drops a few
                // candidates (it considers them "obviously irrelevant"
                // and omits rather than tail-sorts) and very rarely
                // duplicates an index. We treat the LLM's partial
                // order as authoritative for the indices it DID
                // include, then append any missing indices in their
                // original order at the end. Duplicates are dropped.
                // Out-of-range indices are skipped.
                let n = results.len();
                let mut seen = vec![false; n];
                let mut ordered_idx: Vec<usize> = Vec::with_capacity(n);
                for &i in &order {
                    if i >= n || seen[i] {
                        continue;
                    }
                    seen[i] = true;
                    ordered_idx.push(i);
                }
                for i in 0..n {
                    if !seen[i] {
                        ordered_idx.push(i);
                    }
                }
                let mut out: Vec<Option<SearchResult>> =
                    results.into_iter().map(Some).collect();
                ordered_idx
                    .into_iter()
                    .map(|i| out[i].take().expect("validated above"))
                    .collect()
            }
            Err(e) => {
                tracing::warn!(error = %e, "LlmReranker call failed; falling back to input order");
                results
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_ranking_parses() {
        let s = r#"{"ranking":[3,1,0,2]}"#;
        let r: RawRanking = serde_json::from_str(s).unwrap();
        assert_eq!(r.ranking, vec![3, 1, 0, 2]);
    }
}
