use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ProviderId(pub String);

impl ProviderId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A fully-qualified model handle: provider + model id, optionally pinned
/// to a snapshot.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: ProviderId,
    pub model: String,
    pub snapshot: Option<String>,
}

impl ModelRef {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: ProviderId::new(provider),
            model: model.into(),
            snapshot: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub context_window: u32,
    pub max_output_tokens: Option<u32>,
    pub knowledge_cutoff: Option<chrono::NaiveDate>,
    pub deprecated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Region {
    UsEast,
    UsWest,
    EuCentral,
    ApSoutheast,
    Other,
}

/// A single inference request. Provider-specific extensions live behind the
/// typed `Ext` parameter — see `Provider::build_request`.
#[derive(Clone, Debug)]
pub struct Call<Ext = ()> {
    pub model: ModelRef,
    pub messages: Vec<Message>,
    pub tools: Vec<crate::tools::ToolId>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop: Vec<String>,
    pub ext: Ext,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Image { mime: String, bytes: Vec<u8> },
    ToolCall { id: String, name: String, input: serde_json::Value },
    ToolResult { id: String, output: serde_json::Value },
}
