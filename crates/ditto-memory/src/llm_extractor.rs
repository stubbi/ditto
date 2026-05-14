//! LLM-driven `Extractor`.
//!
//! Calls an OpenAI-compatible chat-completions endpoint (default: OpenRouter)
//! to extract structured (subject, relation, object) triples from an
//! event's content. The model is asked to return JSON; we validate
//! against a small schema and turn each triple into a `ProposedFact`.
//!
//! Why an LLM extractor unblocks the rest of the system: `RuleExtractor`
//! only covers four hand-coded relations (`lives_in`, `moved_to`,
//! `works_at`, `allergic_to`). Real conversational data — LoCoMo, real
//! agent tasks — speaks in thousands of relations that don't fit a
//! pattern matcher. Once arbitrary triples land in the NC-graph, the
//! KG-leg of retrieval starts pulling its weight on multi-hop and
//! temporal-update queries.
//!
//! Failure model: any failure (network, parse, refusal) returns
//! `Extraction::empty()`. The dream cycle treats that as "no facts to
//! commit" — semantically equivalent to a turn that didn't trip any
//! pattern. Errors are logged via `tracing::warn` so quality regressions
//! are observable, but they do not abort the dream sweep.

use std::time::Duration;

use async_trait::async_trait;
use ditto_core::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::extractor::{Extraction, Extractor, ProposedFact};

#[derive(Debug, thiserror::Error)]
pub enum LlmExtractorError {
    #[error("env var missing: {0}")]
    EnvMissing(&'static str),
    #[cfg(feature = "embedders-http")]
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("decode: {0}")]
    Decode(String),
}

/// LLM-backed extractor. Cheap to clone — `reqwest::Client` is internally
/// reference-counted.
#[cfg(feature = "embedders-http")]
#[derive(Clone)]
pub struct LlmExtractor {
    api_key: String,
    base_url: String,
    model: String,
    client: reqwest::Client,
}

#[cfg(feature = "embedders-http")]
impl LlmExtractor {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "openai/gpt-4o-mini".into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(45))
                .build()
                .expect("reqwest builder"),
        }
    }

    /// Reads `OPENROUTER_API_KEY` from env. Targets OpenRouter for both
    /// auth and routing.
    pub fn from_env_openrouter() -> Result<Self, LlmExtractorError> {
        let key = std::env::var("OPENROUTER_API_KEY")
            .map_err(|_| LlmExtractorError::EnvMissing("OPENROUTER_API_KEY"))?;
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

    fn build_messages(content: &str) -> Vec<Value> {
        let system = SYSTEM_PROMPT;
        let user = format!(
            "Extract factual triples from this event content. \
             If no factual triples are present, return an empty list.\n\
             \n\
             Content:\n```\n{content}\n```\n\
             \n\
             Return a JSON object: {{\"facts\": [{{...}}]}}. Each fact must \
             have: subject (string), relation (snake_case string), object \
             (string), confidence (0..1 float), supersedes_prior (bool).",
        );
        vec![
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": user}),
        ]
    }

    async fn call_openrouter(&self, content: &str) -> Result<RawExtraction, LlmExtractorError> {
        let body = json!({
            "model": self.model,
            "messages": Self::build_messages(content),
            "temperature": 0.0,
            "max_tokens": 512,
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
            .ok_or_else(|| LlmExtractorError::Decode("missing choices[0].message.content".into()))?;
        let raw: RawExtraction = serde_json::from_str(content)
            .map_err(|e| LlmExtractorError::Decode(format!("parse: {e}; raw={content}")))?;
        Ok(raw)
    }
}

const SYSTEM_PROMPT: &str = "You extract factual triples from short \
conversational utterances. A triple is (subject, relation, object) where \
subject and object are entity names and relation is a snake_case verb or \
relational predicate (lives_in, works_at, owns, prefers, met_at, \
born_in, married_to, friend_of, traveled_to, studied, allergic_to, \
diagnosed_with, etc.). Be conservative: do not invent facts not stated. \
Resolve pronouns to named subjects when context makes it unambiguous; \
otherwise use the speaker's name as subject. Use `supersedes_prior=true` \
ONLY for facts that explicitly replace a prior value of the same \
(subject, relation) — e.g., 'moved to Berlin' replaces a prior \
lives_in. Most facts are additive (`supersedes_prior=false`). Return at \
most 5 triples per utterance.";

#[derive(Debug, Deserialize, Serialize)]
struct RawExtraction {
    facts: Vec<RawFact>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RawFact {
    subject: String,
    relation: String,
    object: String,
    #[serde(default = "default_confidence")]
    confidence: f32,
    #[serde(default)]
    supersedes_prior: bool,
}

fn default_confidence() -> f32 {
    0.7
}

#[cfg(feature = "embedders-http")]
#[async_trait]
impl Extractor for LlmExtractor {
    async fn extract(&self, event: &Event) -> Extraction {
        let Some(content) = content_of(event) else {
            return Extraction::empty();
        };
        if content.trim().is_empty() {
            return Extraction::empty();
        }
        match self.call_openrouter(&content).await {
            Ok(raw) => Extraction {
                facts: raw
                    .facts
                    .into_iter()
                    .filter(|f| !f.subject.is_empty() && !f.relation.is_empty() && !f.object.is_empty())
                    .map(|f| {
                        let mut pf = ProposedFact::new(&f.subject, &f.relation, &f.object)
                            .with_confidence(f.confidence);
                        if f.supersedes_prior {
                            pf = pf.supersedes();
                        }
                        pf
                    })
                    .collect(),
            },
            Err(e) => {
                tracing::warn!(error = %e, "LlmExtractor call failed; returning empty");
                Extraction::empty()
            }
        }
    }
}

fn content_of(event: &Event) -> Option<String> {
    match &event.payload {
        Value::Object(map) => map
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_extraction_parses_minimal_json() {
        let s = r#"{"facts":[{"subject":"alice","relation":"lives_in","object":"berlin"}]}"#;
        let r: RawExtraction = serde_json::from_str(s).unwrap();
        assert_eq!(r.facts.len(), 1);
        assert_eq!(r.facts[0].subject, "alice");
        assert!(!r.facts[0].supersedes_prior);
        assert!((r.facts[0].confidence - default_confidence()).abs() < 1e-6);
    }

    #[test]
    fn raw_extraction_accepts_full_form() {
        let s = r#"{"facts":[{"subject":"alice","relation":"moved_to","object":"tokyo","confidence":0.9,"supersedes_prior":true}]}"#;
        let r: RawExtraction = serde_json::from_str(s).unwrap();
        assert!(r.facts[0].supersedes_prior);
        assert!((r.facts[0].confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn empty_facts_round_trip() {
        let s = r#"{"facts":[]}"#;
        let r: RawExtraction = serde_json::from_str(s).unwrap();
        assert!(r.facts.is_empty());
    }
}
