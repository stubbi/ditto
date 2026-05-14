use serde::{Deserialize, Serialize};

/// Per-call cost telemetry.
///
/// Honest line items beat a single `usd` number. Anthropic's prompt-caching
/// pricing has the 10% / 125% cache-read / cache-write modifiers; reasoning
/// tokens on o1, extended-thinking, and R1 bill separately; OpenRouter charges
/// a 5.5% credit-purchase fee that incumbents hide. Surface them explicitly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CallCost {
    pub input_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub output_tokens: u32,
    pub reasoning_tokens: u32,
    pub usd: f64,
    pub usd_breakdown: CostBreakdown,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub input_usd: f64,
    pub cache_read_usd: f64,
    pub cache_write_usd: f64,
    pub output_usd: f64,
    pub reasoning_usd: f64,
    pub openrouter_fee_usd: f64,
}

impl CostBreakdown {
    pub fn total(&self) -> f64 {
        self.input_usd
            + self.cache_read_usd
            + self.cache_write_usd
            + self.output_usd
            + self.reasoning_usd
            + self.openrouter_fee_usd
    }
}
