use crate::model::ProviderId;
use crate::Error;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod subscription;

/// Handle that a `Provider::stream` call uses to mint a fresh access token.
///
/// The agent process never holds raw API keys directly — every authenticated
/// request goes through this handle, which either reads from a `keyring`
/// entry (API keys) or runs a refresh against a `SubscriptionBackend`.
#[derive(Clone)]
pub struct AuthHandle {
    pub provider: ProviderId,
    inner: Arc<dyn AuthSource>,
}

impl AuthHandle {
    pub fn new(provider: ProviderId, inner: Arc<dyn AuthSource>) -> Self {
        Self { provider, inner }
    }

    pub async fn token(&self) -> Result<AccessToken, Error> {
        self.inner.token().await
    }
}

#[async_trait]
pub trait AuthSource: Send + Sync {
    async fn token(&self) -> Result<AccessToken, Error>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessToken {
    pub token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub token_type: TokenType,
    /// Extra request headers required alongside the bearer token (Copilot's
    /// `Editor-Version`, `Copilot-Integration-Id`; Anthropic-on-Vertex's
    /// `anthropic-version`).
    pub aux_headers: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    ApiKey,
    Bearer,
    AwsSigv4,
    GoogleAdc,
}

/// Subscription-based auth (OAuth flows redeemed against a user's paid
/// subscription rather than per-request billing on an API key).
#[async_trait]
pub trait SubscriptionBackend: Send + Sync {
    fn provider(&self) -> ProviderId;
    fn policy_status(&self) -> PolicyStatus;
    fn rate_limit_class(&self) -> RateLimitClass;

    async fn ensure_token(&self) -> Result<AccessToken, Error>;
    async fn login(&self, kind: LoginKind) -> Result<LoginOutcome, Error>;
    async fn refresh(&self) -> Result<AccessToken, Error>;
    fn revoke(&self) -> Result<(), Error>;
}

/// Whether a subscription backend is contractually clean, gray, or actively
/// banned. This enum is the architectural acknowledgement that some
/// providers' terms of service drift faster than software ships.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStatus {
    /// Vendor publicly endorses 3rd-party clients (Copilot, Gemini free tier).
    Allowed,
    /// Documented or tolerated in practice but not formally endorsed (Codex).
    GreyArea,
    /// Vendor has actively blocked 3rd-party use. As of 2026-04-04, Anthropic
    /// took this stance for Claude Code OAuth against unaffiliated clients.
    /// Backends with this status are disabled unless the user passes
    /// `--accept-policy-risk` explicitly.
    EnforcedBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RateLimitClass {
    /// Free tier — quotas measured in requests/minute and requests/day.
    Free,
    /// Paid subscription — generous but still capped (Copilot Pro, Codex Plus).
    Paid,
    /// Pay-per-token API key — effectively unlimited.
    PayAsYouGo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginKind {
    DeviceCode,
    InstalledApp,
    BrowserRedirect,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LoginOutcome {
    Success {
        token: AccessToken,
    },
    /// Device-flow / installed-app intermediate state — the CLI prints the
    /// user-code and verification URL.
    Pending {
        user_code: String,
        verification_uri: String,
        expires_in_seconds: u64,
    },
}
