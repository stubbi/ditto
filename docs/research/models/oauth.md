# Subscription-Native OAuth for LLM APIs: Forensic Research for Ditto

**Date:** 2026-05-14
**Author:** Research agent for Ditto (github.com/stubbi/ditto), Bet 3 wedge
**Scope:** Every public OAuth flow an agent harness can legitimately use today to consume LLM capacity from a user's existing consumer subscription (Claude Pro/Max, ChatGPT Plus/Pro, GitHub Copilot, Gemini, M365 Copilot, Cursor, etc.). What's technically possible, what's contractually permitted, what got banned, and what Ditto should actually ship.

---

## 1. Executive summary — the three flows Ditto must ship

The terrain has shifted dramatically since the original Bet 3 framing. The single most-requested feature ("let me use my Claude Code subscription") is now contractually prohibited and technically blocked. The honest answer to those gbrain / openhuman / hermes-agent issues is no longer "let me build it" — it's "Anthropic killed it on April 4, 2026, here's what we ship instead."

**Recommended stack for Ditto, ranked:**

1. **GitHub Copilot OAuth (device flow)** — the only subscription-OAuth flow that is (a) explicitly designed for third-party clients via a public SDK, (b) currently shipping in continue.dev, aider, opencode, litellm, and a dozen cline forks without C&D, and (c) gives access to Claude Opus 4.7, GPT-5.5, Gemini 2.5 Pro, and o-series models under one $10–$39/mo subscription. This is the wedge.
2. **OpenAI Codex "Sign in with ChatGPT" (PKCE)** — officially documented at `developers.openai.com/codex/auth`, OpenAI itself shipped it in Cline ("Bring your ChatGPT subscription to Cline" blog, official partnership), and OpenAI has not moved against opencode/Codex auth bridges. Lower risk than Anthropic, but still gray for arbitrary 3rd-party harnesses — see §4.
3. **Gemini CLI "Login with Google" (Code Assist OAuth)** — Google's own gemini-cli ships the OAuth flow, the client_id/secret are in open source, the free-tier quota (60 rpm / 1000 rpd) actually works for individual users, and reuse in third-party tools is a documented gray area Google hasn't enforced against.

**What Ditto should NOT ship as first-class:** Anthropic Claude Code OAuth. Technically possible (Meridian / OCP / opencode-claude-auth still work in May 2026 because they piggyback on the official SDK), but legally toxic post-April-4 and a guaranteed lifecycle ticking time bomb. Ship as a BYO-credential plug-in pattern where Ditto never hosts, never proxies, never UX-installs the flow — the user pastes their `CLAUDE_CODE_OAUTH_TOKEN` and accepts the risk.

**What Ditto must support as second-class:** Anthropic Console API key, OpenAI API key, Azure OpenAI / Entra ID OBO, Vertex AI service accounts, Bedrock IAM. These are the "real" billing relationships and the only auth paths Anthropic explicitly endorses for non-Anthropic products.

---

## 2. Per-provider OAuth flow detail

### 2.1 Anthropic Claude Code OAuth

**Status as of 2026-05-14: contractually banned for third-party tools; technically still functional via SDK piggybacking.**

The reverse-engineered flow (akashmohan.com/writings/claude-code-oauth, fully consistent with what `opencode-anthropic-auth` and `meridian` use):

| Field | Value |
|---|---|
| Authorization endpoint | `https://claude.ai/oauth/authorize` |
| Token endpoint | `https://console.anthropic.com/v1/oauth/token` |
| API base | `https://api.anthropic.com/v1/messages` |
| Client ID | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` (public client, no secret) |
| Flow | PKCE (S256), localhost callback, fallback paste-code for WSL/SSH |
| Scopes | `user:profile user:inference` |
| Access token | Opaque, prefix `sk-ant-oat01-`, lifetime 28800s (8h) |
| Refresh token | Opaque, prefix `sk-ant-ort01-`, **rotates on every refresh** |
| Required header | `anthropic-beta: oauth-2025-04-20,claude-code-20250219` |
| Required header | `anthropic-version: 2023-06-01` |
| Required system prompt | Must include `"You are Claude Code, Anthropic's official CLI for Claude."` |
| Auth header | `Authorization: Bearer <access_token>` (NOT `X-Api-Key`) |

Storage on the official CLI: macOS Keychain; Linux/Windows `~/.claude/.credentials.json` mode 0600. Refresh handled proactively 5 min before expiry, reactively on 401.

**The crackdown (cited in detail in §4):** Anthropic updated consumer terms on Feb 20, 2026, deployed server-side validation through February–March, and enforced fully on April 4, 2026 at 12:00 PM PT. Boris Cherny (Head of Claude Code) on X: *"Our subscription model was not designed for the usage patterns of these third-party tools."* The official docs at `code.claude.com/docs/en/authentication` now state explicitly that the Claude Code OAuth flow is "intended exclusively for Claude Code and Claude.ai" — using OAuth tokens "in any other product, tool, or service — including the Agent SDK — is not permitted."

**What still works (technically, not legally):** Meridian (rynfar/meridian, v1.42.1 as of 2026-05-06) and OCP (Open Claude Proxy) work because they don't impersonate the CLI — they shell out to the actual official `@anthropic-ai/claude-code` SDK's `query()` function, which authenticates with whatever the user's `claude login` left in `~/.claude/`. This means Anthropic's server-side fingerprinting (which checks for the official user-agent, beta header, and system prompt) sees the official SDK and lets the request through. This is a load-bearing technicality; it can break at any time and is still a ToS violation for the user. The April 4 enforcement specifically targeted **header-spoofing proxies**, not SDK-piggybacking, but Anthropic's enforcement language ("may do so without prior notice") covers both.

**Claude Max specifics:** Same OAuth flow; Max ($100/$200) gets 5x and 20x the Pro ($20) usage allowance; access to Opus is gated on Max ($100+) and Team/Enterprise plans. Starting June 15, 2026, **Agent SDK usage will draw from a separate monthly Agent SDK credit** (per code.claude.com docs), explicitly separating interactive Claude Code use from programmatic use — this is the policy infrastructure that lets Anthropic price 3rd-party-style usage differently going forward.

**Long-lived tokens (`claude setup-token`):** Anthropic actually does offer a 1-year OAuth token for CI/scripts (`CLAUDE_CODE_OAUTH_TOKEN`) — generated via the same OAuth flow, scoped to inference only. **This is the only "legitimate" path** for a user to use their subscription in a non-Anthropic place, and it's framed as "CI pipelines and scripts where browser login isn't available." A Ditto user could paste this into Ditto manually — but Ditto cannot ship the wizard that generates it without violating "interaction with Claude's capabilities... should use API key authentication."

### 2.2 OpenAI Codex / "Sign in with ChatGPT"

**Status: officially supported in OpenAI's own products and an OpenAI-blessed partnership with Cline; gray for arbitrary 3rd parties; no enforcement actions to date.**

OpenAI ships this as a first-class auth path at `developers.openai.com/codex/auth`. The flow is browser-based OAuth (PKCE) with optional device-code mode, and tokens are JWTs containing the user's account/tier info. Tokens refresh automatically; default cache at `~/.codex/auth.json` (plaintext) or OS credential store.

OpenAI explicitly partnered with Cline (cline.bot blog, "Introducing OpenAI Codex OAuth") to bring "Sign in with OpenAI" to a third-party tool — the first such partnership for any major provider. Users with ChatGPT Plus ($20), Pro ($200), Business ($30/user), or Enterprise plans get tier-mapped access to:

- gpt-5.2-codex (agentic coding)
- gpt-5.2 (general)
- gpt-5-mini
- o3, o4-mini (reasoning)

Pro tier gets **5x the Codex usage of Plus** (per VentureBeat coverage of the $200 ChatGPT Pro tier).

**Third-party use status:** Tools like `numman-ali/opencode-openai-codex-auth` ship the same PKCE flow as the official Codex CLI. The OpenAI Codex GitHub repo has a public discussion (`openai/codex#8338`) asking whether forking/modifying Codex CLI affects ToS when using "Sign in with ChatGPT"; OpenAI has not closed this discussion with a hard "no." Service Terms prohibit "sharing account credentials" and "automated/programmatic extraction" but do not specifically prohibit personal use of one's own subscription in a non-Codex client. The Cline partnership is the strongest signal that OpenAI is comfortable with this model — provided the user is signing in themselves and using their own quota.

**Key technical difference vs Anthropic:** Codex tokens are JWTs (so a client can inspect tier/scope without round-tripping), and refresh is silent and high-frequency. Endpoint is the ChatGPT backend, NOT `api.openai.com` — the ChatGPT subscription does not grant API quota on the developer platform. They are separate billing relationships.

**The "use ChatGPT Plus as an API" question:** No, not via api.openai.com. ChatGPT Plus and the OpenAI API are separate products with separate billing (same as Perplexity Pro / Perplexity API). The ONLY legitimate way to consume the ChatGPT subscription programmatically is via the Codex auth path described above, which routes to the ChatGPT backend (not platform.openai.com). Projects like `llm-openai-codex` (PyPI) wrap exactly this.

### 2.3 GitHub Copilot OAuth — the workhorse

**Status: explicitly designed for third-party integrations via the public Copilot SDK, broad OSS adoption, no public enforcement against personal use in third-party agents, abuse-detection limits exist.**

This is the most mature subscription-OAuth flow in the industry because Copilot has been around longest and GitHub has been gradually opening it up. As of March 2026, GitHub formalized this with the GitHub Generative AI Services Terms, replacing the older Copilot Product Specific Terms.

**Two-stage token model:**

1. **GitHub OAuth device flow** — at `github.com/login/device`, user enters a one-time code; client receives a long-lived `gho_*` GitHub token with `copilot` scope (and `read:user`).
2. **Copilot token exchange** — client calls `https://api.github.com/copilot_internal/v2/token` (or the newer `https://api.githubcopilot.com/token` endpoint) with the GitHub token; receives a **30-minute Copilot bearer token** plus expiry epoch.
3. **Inference** — calls `https://api.githubcopilot.com/chat/completions` (OpenAI-compatible) or `/v1/messages` for Anthropic-shape responses, depending on integration. Token refresh: when the Copilot token nears 30-min expiry, client re-exchanges its long-lived GitHub token.

**Required headers (this is where most reverse-eng tools break):**

```
Authorization: Bearer <copilot_token>
Editor-Version: vscode/1.95.0          # or jetbrains/2024.3, etc.
Editor-Plugin-Version: copilot-chat/0.20.0
Copilot-Integration-Id: vscode-chat    # registered IDs are gatekept
OpenAI-Intent: conversation-panel
```

Missing `Editor-Version` returns HTTP 400 "missing Editor-Version header for IDE auth" (litellm #18475, openclaw #58056). `Copilot-Integration-Id` is the lever GitHub uses to track which clients are calling — values like `vscode-chat`, `copilot-cli`, `intellij-chat` are official; tools like `copilot-api` and `litellm` set one of these. **GitHub knows this, hasn't moved against it.**

**Models in Copilot, May 2026 (per docs.github.com/en/copilot/reference/ai-models/supported-models):**

- Anthropic: Claude Haiku 4.5, Sonnet 4 / 4.5 / 4.6, Opus 4.5 / 4.6 / 4.7
- OpenAI: GPT-5, GPT-5-mini, GPT-5.5, o-series
- Google: Gemini 2.5 Pro
- GPT-4.1, GPT-5.2, GPT-5.2-Codex closing down 2026-06-01

**Plan tier → quota:**

- Copilot Free: 50 premium req/mo, 2000 completions/mo, Haiku 4.5 + GPT-5 mini
- Copilot Pro ($10): 300 premium req/mo + $0.04 overage
- Copilot Pro+ ($39): 1500 premium req/mo, full frontier including Opus 4.7
- Copilot Business ($19/user): 300 premium req/user, org policy
- Copilot Enterprise: as Business + custom models

**Critical 2026 change:** Starting June 1, 2026, Copilot moves from request-based to usage-based billing. Starting April 20–22, 2026, new sign-ups for Pro/Pro+/student/Business-on-free are paused due to abuse during the Codex/Claude-Code-ban migration surge (The Register, April 20, 2026).

**Abuse-detection:** GitHub's terms explicitly call out "scripted interactions," "deliberately unusual or strenuous usage," and "multiple accounts to circumvent usage limits." Users have been suspended for what looks like agent-style consumption (community discussion #160013, #161697). For Ditto, the rule is: **respect the per-plan request budget; surface remaining-quota headers to the user; do not parallel-fan-out beyond what a human IDE user would.**

### 2.4 Google Gemini / Code Assist

**Status: Google ships the OAuth flow in open source with embedded credentials; third-party use is technically trivial; Google quietly tolerates it but is increasingly flagging "abuse-detection" on heavy automated use.**

The gemini-cli OAuth flow lives in plain sight at `github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/oauth2.ts`. The OAuth client ID and "secret" are committed to the public repo with a comment that "in this context, the client secret is obviously not treated as a secret" — i.e., Google treats this as a public-client desktop install, not a real secret.

Flow:

1. Browser-based "Login with Google" → standard Google OAuth2 with the embedded installed-app client.
2. Token exchange against `https://oauth2.googleapis.com/token`.
3. Inference via the Code Assist API endpoint (`cloudcode-pa.googleapis.com`) — calls `loadCodeAssist` to detect tier (FREE / STANDARD / PAID) and routes to gemini-2.5-flash / gemini-2.5-pro.

**Free tier:** 60 req/min, 1000 req/day with a personal Google account, no billing setup required. This is **the only major provider that gives genuinely free, no-credit-card LLM access via OAuth** — and Google ships it as a feature, not a bug.

**Google One AI Premium / Google AI Pro ($19.99):** Does NOT cleanly grant API access. There's an ongoing gemini-cli bug (#24517, #24747) where AI Pro subscribers see 403 PERMISSION_DENIED on the Code Assist API path despite the CLI correctly identifying their subscription. Google support has bounced these users between Google One and Cloud teams. **As of May 2026 this is broken.** AI Pro users who want Gemini 2.5 Pro programmatically essentially have to pay separately on Vertex AI or AI Studio API.

**Third-party reuse:** Tools like `cline/cline#4495` and `RooCodeInc/Roo-Code#5134` discuss porting the Gemini CLI OAuth provider verbatim. Google has not enforced against this — but support has used "may have been flagged by an abuse detection system" as the cooling-off response. This is the cleanest "free OAuth" flow today but quotas are stingy and the path to paid is unclear.

### 2.5 Microsoft 365 Copilot / Azure OpenAI

**Status: not really a "subscription OAuth" story — it's Entra ID enterprise auth. Genuinely usable for third-party agents in an org context, but not the consumer wedge.**

Azure OpenAI uses Entra ID (formerly Azure AD) with two patterns relevant to Ditto:

1. **Client credentials grant** — service principal calls `cognitiveservices.azure.com` directly. This is API-key-style for org-owned agents. Not a "subscription" play.
2. **On-Behalf-Of (OBO)** — Ditto registers as an app in the customer's Entra tenant, the user signs in with their Entra account, Ditto exchanges the user's token for an Azure OpenAI access token. The downstream API call carries the user's identity and permissions.

For M365 Copilot specifically, the same OBO pattern lets agents call the M365 Copilot API on behalf of a signed-in user — but this consumes M365 Copilot Studio quota at the org level, not the user's personal Copilot subscription. Microsoft Agent 365 also issues each AI agent its own Entra Agent ID for governance.

**Verdict for Ditto:** This is the "enterprise" auth flow, not the consumer-subscription wedge. Ship it in v2 for enterprise customers; don't try to make it the consumer onboarding. The OBO flow requires per-tenant app registration consent, which kills the one-click UX.

### 2.6 Cursor Pro

**Status: no public OAuth path; one OSS project (`Nomadcxx/opencode-cursor`) proxies Cursor's internal API; high churn risk.**

Cursor's $20 Pro plan ($20 of "included usage" tokens) has no documented external API or OAuth flow for consuming that quota from outside the Cursor editor. `Nomadcxx/opencode-cursor` (and `anyrobert/cursor-api-proxy`) reverse-engineer Cursor's internal API and rotate sessions, but this is unambiguous ToS violation territory and has historically been quickly broken by Cursor with each update. **Do not ship.** Flag as user-pluggable BYO at most.

### 2.7 Perplexity Pro / Mistral Le Chat / DeepSeek / Grok

**Perplexity Pro ($20):** Removed the free $5/mo API credits on Feb 12, 2026. Pro and API are formally separate billing relationships now. No OAuth bridge.

**Mistral Le Chat Pro:** No public OAuth path; La Plateforme API is separate.

**DeepSeek consumer:** No OAuth subscription flow; deepseek.com chat and platform API are separate.

**xAI / Grok:** X Premium+ includes Grok web UI access; no OAuth path to consume from outside grok.com or X clients.

**Verdict:** None of these have a usable subscription-OAuth path. Skip in v1; revisit if any of them ship a Copilot-style integration ID model.

---

## 3. Implementation patterns

### 3.1 Flow choice

- **PKCE + localhost callback:** Anthropic, OpenAI Codex, Google Gemini CLI. Works on desktop with a browser. Falls back to paste-code mode under WSL2 / SSH / containers.
- **Device code flow:** GitHub Copilot is the canonical example; also available as opt-in for Codex. **This is the best fallback for any headless / Docker / remote scenario** and Ditto should support it for every provider that exposes it.
- **Deep link / custom URL scheme (`ditto://oauth/callback`):** worth registering for desktop variants of Ditto but not strictly required since localhost callback works.

### 3.2 Token storage

The OSS-tool consensus (and what Anthropic / Codex / Copilot CLIs themselves do):

- macOS: Keychain via `security` framework
- Windows: Credential Manager via `wincred`
- Linux: Secret Service (libsecret / gnome-keyring) with fallback to mode-0600 file in `~/.config/ditto/credentials.json`
- Cross-platform Rust: the `keyring` crate (Ditto is presumably Rust given the github.com/stubbi/ditto stack); Go: `zalando/go-keyring`; Node: `@napi-rs/keyring`

Note: Codex defaults to plaintext `~/.codex/auth.json` and is widely criticized for it; opencode ships the same plaintext model in `~/.local/share/opencode/auth.json` and has an open issue (#4318) requesting keyring support. **Ditto should default to OS keyring and treat plaintext as opt-in.**

### 3.3 Refresh handling

Three patterns observed:

- **Anthropic Claude Code:** rotate refresh token on every use; refresh 5 min before expiry proactively, also on 401 reactively. Store latest refresh token immediately or you lose the chain.
- **OpenAI Codex:** silent refresh from JWT; no rotation; long-lived.
- **GitHub Copilot:** two-stage. The 30-min Copilot bearer is refreshed silently from the long-lived GitHub token; the GitHub token never refreshes (revoke-only).

**Backoff:** On refresh failure (5xx or network), exponential backoff with jitter, max 3 retries before falling back to interactive re-auth prompt. Never silently fail-open.

### 3.4 Quota / rate-limit headers

Each provider exposes different telemetry:

- Anthropic: `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-tokens-remaining`, `anthropic-ratelimit-tokens-reset`
- OpenAI: `x-ratelimit-remaining-requests`, `x-ratelimit-remaining-tokens`, `x-ratelimit-reset-tokens`
- GitHub Copilot: returns `x-ratelimit-limit`, `x-ratelimit-remaining`, but also surfaces "premium request" quota via `/copilot_internal/user` endpoint (not headers)
- Gemini: `x-goog-api-quota-remaining` (when present), tier shows via `loadCodeAssist` once

Ditto should normalize these into a single internal `QuotaSnapshot { remaining_requests, remaining_tokens, reset_at }` and surface it in the CLI prompt bar (continue.dev and opencode both do this; users love it).

### 3.5 Streaming

All four working flows (Copilot, Codex, Anthropic, Gemini) speak SSE on inference endpoints. The bearer/OAuth token is set once per request; no per-chunk signing. No special considerations beyond standard SSE handling.

---

## 4. ToS & legal landscape

The 2026 industry split is now crisp:

| Provider | Personal use in 3rd-party client | Redistribute as feature in product |
|---|---|---|
| Anthropic Claude Pro/Max | **Prohibited** (Feb 20 terms, April 4 enforced) | **Prohibited** |
| OpenAI ChatGPT/Codex | Gray, no enforcement, Cline partnership signals tolerance | Gray; ask for partnership |
| GitHub Copilot | **Permitted** via Copilot SDK + Integration ID model | **Permitted** with registered Integration ID |
| Google Gemini Code Assist | Gray; OAuth credentials in open source signal toleration | Gray; abuse-detection risk |
| Azure OpenAI / M365 | Permitted via Entra ID OBO with admin consent | Permitted with tenant consent |
| Cursor Pro | **Prohibited** (no docs, scraping required) | **Prohibited** |
| Perplexity / Mistral / DeepSeek / Grok | No flow exists | No flow exists |

**The Anthropic terms (Feb 17–20, 2026), verbatim from the policy and AndyMik90/Aperant#1871 discussion:**

> "OAuth authentication (used with Free, Pro, and Max plans) is intended exclusively for Claude Code and Claude.ai."

> "Use of OAuth tokens obtained via Claude Free, Pro, or Max accounts in any other product, tool, or service — including the Agent SDK — is not permitted."

> "Developers building products or services that interact with Claude's capabilities... should use API key authentication through Claude Console or a supported cloud provider."

> "Anthropic reserves the right to take measures to enforce these restrictions and may do so without prior notice."

This is unambiguous. Any product that bundles a Claude OAuth flow is violating the consumer terms on the user's behalf, exposing the user to account suspension, and risking direct legal action against the product.

**Anthropic enforcement timeline:**

- Jan 9, 2026: First temporary lockout (Thariq Shihipar tweets enforcement coming)
- Feb 14, 2026: OpenClaw founder Peter Steinberger joins OpenAI (read the room)
- Feb 17–20, 2026: Terms updated, language hardened
- Feb–Mar 2026: Server-side validation deployed
- April 4, 2026 12:00 PM PT: Hard enforcement, OpenClaw and most opencode plugins broken
- June 15, 2026 (upcoming): Agent SDK on subscription draws from separate monthly Agent SDK credit (per `code.claude.com/docs/en/authentication`)

**The "personal use via my own client" defense doesn't survive contact with Anthropic's terms.** Other providers are friendlier:

- **OpenAI's** Service Terms prohibit credential sharing and automated extraction but **do not currently prohibit using your own Codex login from a non-Codex client**. The `openai/codex#8338` discussion was left open. The Cline partnership signals affirmative tolerance.
- **GitHub Copilot's** terms (March 5, 2026 Generative AI Services Terms) are notably structured around abuse-detection and Integration IDs rather than client exclusivity — i.e., they regulate behavior, not which binary calls the API. This is by design.
- **Google's** AI Pro terms are silent on third-party clients; the embedded OAuth credentials in gemini-cli are a tacit "go ahead" for personal use.

### The product-redistribution line

"Personal use via my own client" (user installs Ditto, signs in themselves, Ditto talks to their account) is the strongest defense across all four working flows. "Hosted SaaS Ditto where users sign in and we proxy their quota" is unambiguously over the line for Anthropic and gray for everyone else. **Ditto should be local-first on every OAuth path.** Tokens never leave the user's machine. No Ditto cloud component should ever touch a user's subscription OAuth token. This is non-negotiable both legally and as a customer-trust matter.

---

## 5. Community & OSS reality

**What's currently shipping that works (May 2026):**

- **continue.dev:** GitHub Copilot OAuth (works); Anthropic OAuth quietly removed post-April-4; Codex sign-in shipped.
- **aider:** GitHub Copilot via `aider.chat/docs/llms/github.html`, otherwise API keys.
- **opencode (sst/opencode and forks):** Bundled Claude Pro/Max plugin was **explicitly removed in v1.3.0** per opencode.ai docs ("Previous versions of OpenCode came bundled with plugins allowing Claude Pro/Max use. That is no longer the case as of 1.3.0"). Still ships GitHub Copilot, ChatGPT Plus/Pro Codex, GitLab Duo, DigitalOcean. Third-party plugins for Claude (opencode-claude-auth, opencode-with-claude via Meridian) still exist but are user-installed at user's risk.
- **Cline:** Officially partnered with OpenAI for Codex sign-in. Has GitHub Copilot OAuth. Roo-Code (fork) closed the Claude Pro/Max OAuth request as not-planned (#4799).
- **LiteLLM:** GitHub Copilot first-class provider; Claude Code Max subscription tutorial exists but post-April-4 is effectively dead.
- **Meridian (rynfar):** Still shipping (v1.42.1 May 6, 2026), still works via SDK piggybacking, but legally compromised.

**What got cease-and-desisted or technically killed:**

- **OpenClaw:** primary target of April 4 enforcement, named in VentureBeat and apiyi.com coverage
- **NanoClaw, "ClaudeMax" variants:** named in policy enforcement discussions
- Header-spoofing proxies generally: blocked by Anthropic server-side fingerprinting

**HN sentiment (item 47069299):**

The discourse was overwhelmingly hostile to Anthropic's move ("Anthropic Just Locked the Door" / "burning the loyalty the model team is earning") but also realistic: top comments noted that consumer subscriptions are massively subsidized vs API rates ($200/mo Max running $1k–$5k of compute is unsustainable). Two consistent migration paths emerged: (a) Kimi K2.5 + opencode + local models, (b) ChatGPT Pro + Codex + Cline. Several authors of OSS tools stated explicitly they'd remove or feature-flag Anthropic OAuth support to avoid ToS exposure.

**Twitter / X positioning:**

Boris Cherny's post is the load-bearing public statement. Anthropic engineer Thariq Shihipar foreshadowed enforcement in January. Continue.dev, opencode (Adam Wathan / Dax), and Cline all posted post-enforcement statements pivoting to API-key-only or Copilot-routed Claude access.

---

## 6. Ditto recommendations

### Concrete stack, ranked

1. **GitHub Copilot OAuth as the primary onboarding wedge.** Implement device-code flow, register a `Copilot-Integration-Id` (apply to GitHub for an official one — it's possible), expose Claude Opus 4.7, GPT-5.5, Gemini 2.5 Pro under one auth path. Surface remaining premium-request quota in the CLI prompt bar. Respect rate limits — never parallel-fan-out beyond what a sensible IDE user would do. This is the only flow that's both technically robust and contractually clean.

2. **OpenAI Codex "Sign in with ChatGPT" as the cross-sell.** Implement PKCE flow against OpenAI's documented Codex auth. Map ChatGPT tier → model availability. Reach out to OpenAI for partnership status à la Cline; their official blog signals they're receptive. Plus subscribers get a viable agent-coding experience at $20/mo.

3. **Gemini CLI OAuth for the "free tier on-ramp."** Use the publicly embedded Code Assist client_id, default to free tier (60 rpm / 1000 rpd), let users upgrade to Vertex AI or AI Studio API keys when they outgrow it. This is your "no credit card required, try Ditto in 30 seconds" path.

4. **Claude Console API key as the Anthropic path.** First-class support for `ANTHROPIC_API_KEY` (and Bedrock / Vertex / Foundry env-var fallback). Anthropic's own docs list this as the supported path for "products and services that interact with Claude."

5. **BYO Claude Code OAuth token as an unsupported plug-in.** Document `CLAUDE_CODE_OAUTH_TOKEN` env-var support (Anthropic's own long-lived-token mechanism). The user pastes it; Ditto sends it as `Authorization: Bearer`. **No wizard, no OAuth UI, no in-product "Sign in with Claude" button** — that's what crosses the line. This satisfies the gbrain / openhuman / hermes-agent feature requests without shipping a contractual landmine.

6. **Azure OpenAI / Entra ID OBO for enterprise.** v2 feature. Implement once you have your first enterprise customer with M365 Copilot / Azure OpenAI commitments.

### CLI UX for the auth wizard

```
$ ditto auth login
Which provider do you want to use?
  1. GitHub Copilot         (recommended — $10/mo, 7 frontier models)
  2. ChatGPT (Codex)        ($20/mo, OpenAI models)
  3. Google Gemini          (free tier 60 rpm)
  4. Anthropic Console API  (per-token billing)
  5. Self-hosted / proxy    (advanced)
  6. Bring your own token   (paste CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY, etc.)
```

For each OAuth flow:

- Show the consent URL and a copy-able QR code (terminal QR for mobile auth on dev servers).
- Localhost callback by default; auto-detect WSL/SSH/container and switch to device-code mode.
- After success, print "Stored in macOS Keychain" / "Stored in Linux Secret Service" / "Stored in ~/.config/ditto/credentials.json (mode 0600)".
- Run a single test inference call to validate end-to-end.

### Token rotation policy

- On every successful refresh, immediately overwrite the stored refresh token (critical for Anthropic-style rotation).
- On refresh failure: 3 retries with exponential backoff + jitter (250ms → 1s → 4s), then prompt for re-auth.
- On 401 response: refresh once and retry; if still 401, prompt for re-auth (do not loop).
- Long-lived tokens (`CLAUDE_CODE_OAUTH_TOKEN`, generated GitHub PATs): warn user 30 days before expiry.

### Multi-account support

Required from day 1 for serious developers (work + personal Copilot, multiple Claude Console orgs). Pattern:

```
$ ditto auth list
  github-copilot (default)  jannes@aqora.io
  github-copilot            work@paperclip.inc
  anthropic-console         ditto-prod
  chatgpt                   jannes-personal

$ ditto --profile work ...
```

Profiles map to distinct entries in the keyring. `~/.config/ditto/profiles.toml` indexes them. Mirror `aws --profile` semantics; developers know this.

### Hard rules for Ditto

1. **Never proxy a user's subscription OAuth token through Ditto-hosted infrastructure.** Tokens live on the user's machine.
2. **Never ship a wizard that performs the Anthropic Claude Code OAuth flow on the user's behalf.** Document the BYO-token escape hatch instead.
3. **Always set the right Integration ID / Editor-Version / system prompt** when talking to a provider — these are the provider's tracking mechanisms and lying about them is what got OpenClaw banned. Be Ditto, identified honestly, and respect rate limits.
4. **Surface the quota state in the prompt bar.** The #1 complaint across continue.dev / opencode / Cline communities is "I burned my quota and didn't know."

---

## 7. Citations

All accessed 2026-05-14 unless noted. URLs are absolute.

**Anthropic Claude Code OAuth:**
- https://akashmohan.com/writings/claude-code-oauth — reverse-engineered technical specs
- https://code.claude.com/docs/en/authentication — official auth docs incl. the third-party prohibition
- https://venturebeat.com/technology/anthropic-cracks-down-on-unauthorized-claude-usage-by-third-party-harnesses — VentureBeat coverage of crackdown
- https://help.apiyi.com/en/anthropic-claude-subscription-third-party-tools-openclaw-policy-en.html — timeline and Cherny statement
- https://daveswift.com/claude-trouble/ — user lockout report Feb 2026
- https://daveswift.com/claude-oauth-update/ — token expiry / alternatives
- https://news.ycombinator.com/item?id=47069299 — HN community discourse
- https://github.com/anthropics/claude-code/issues/13770 — anthropic-beta header bug
- https://github.com/AndyMik90/Aperant/issues/1871 — exact policy text + Auto-Claude impact analysis
- https://github.com/badlogic/pi-mono/issues/2751 — OAuth tokens must use Bearer, not X-Api-Key

**Claude Pro/Max OAuth in 3rd-party tools (status):**
- https://github.com/rynfar/meridian — Claude Code SDK bridge, still active May 2026
- https://github.com/ianjwhite99/opencode-with-claude — opencode plugin via Meridian
- https://github.com/griffinmartin/opencode-claude-auth — opencode-claude-auth
- https://github.com/jcubic/opencode-claude-plan — plan to restore Claude in opencode
- https://github.com/RooCodeInc/Roo-Code/issues/4799 — Claude OAuth feature request closed not-planned
- https://github.com/ex-machina-co/opencode-anthropic-auth — opencode anthropic OAuth bridge
- https://dev.to/dtzp555max/use-your-claude-promax-subscription-to-power-openclaw-opencode-cline-and-any-openai-compatible-na0 — OCP project
- https://docs.litellm.ai/docs/tutorials/claude_code_max_subscription — LiteLLM Claude Max tutorial
- https://docs.litellm.ai/docs/tutorials/claude_code_beta_headers — Claude Code anthropic-beta header management

**OpenAI Codex OAuth:**
- https://developers.openai.com/codex/auth — official Codex auth docs
- https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan — Codex + ChatGPT
- https://cline.bot/blog/introducing-openai-codex-oauth — OpenAI x Cline partnership
- https://github.com/numman-ali/opencode-openai-codex-auth — opencode Codex plugin
- https://github.com/openai/codex/discussions/8338 — open ToS discussion for forks
- https://venturebeat.com/orchestration/openai-introduces-chatgpt-pro-usd100-tier-with-5x-usage-limits-for-codex — Pro tier 5x Codex quota
- https://codex.danielvaughan.com/2026/04/01/codex-cli-authentication-flows-credential-management/ — Codex auth flows + credential mgmt
- https://pypi.org/project/llm-openai-codex/0.2.1/ — llm-openai-codex PyPI

**GitHub Copilot OAuth:**
- https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli — official device flow docs
- https://docs.github.com/en/copilot/how-tos/copilot-sdk/set-up-copilot-sdk/github-oauth — GitHub Copilot SDK OAuth
- https://docs.github.com/en/copilot/reference/ai-models/supported-models — current model list (May 2026)
- https://docs.github.com/en/copilot/concepts/billing/copilot-requests — premium request quotas
- https://docs.github.com/en/copilot/get-started/plans — plan tiers and pricing
- https://docs.litellm.ai/docs/providers/github_copilot — LiteLLM Copilot provider
- https://github.com/ericc-ch/copilot-api — reverse-eng Copilot proxy
- https://github.com/caozhiyuan/copilot-api — Copilot proxy fork (Claude Code / Codex / opencode compatible)
- https://thakkarparth007.github.io/copilot-explorer/posts/copilot-internals.html — Copilot internals reverse-eng
- https://bootk.id/posts/copilot/ — Copilot reverse engineering writeup
- https://aider.chat/docs/llms/github.html — aider Copilot integration
- https://github.com/BerriAI/litellm/issues/18475 — Editor-Version header requirement
- https://github.com/openclaw/openclaw/issues/58056 — Editor-Version causes HTTP 400
- https://www.theregister.com/2026/04/20/microsofts_github_grounds_copilot_account/ — Copilot signup pause April 2026
- https://github.com/orgs/community/discussions/160013 — abuse-detection warnings
- https://github.com/orgs/community/discussions/161697 — Copilot suspension example
- https://github.com/orgs/community/discussions/192097 — Copilot Pro account suspension
- https://github.com/customer-terms/github-copilot-product-specific-terms — current Copilot product terms

**Google Gemini / Code Assist:**
- https://google-gemini.github.io/gemini-cli/docs/get-started/authentication.html — gemini-cli auth
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/oauth2.ts — embedded OAuth credentials
- https://github.com/google-gemini/gemini-cli/issues/24517 — AI Premium 403 bug
- https://github.com/google-gemini/gemini-cli/issues/24747 — AI Premium VS Code 403 bug
- https://gemini.google/subscriptions/ — Google AI Pro/Ultra pricing
- https://geminicli.com/docs/resources/quota-and-pricing/ — free tier quotas
- https://github.com/cline/cline/issues/4495 — porting Gemini OAuth to Cline
- https://github.com/RooCodeInc/Roo-Code/issues/5134 — Gemini OAuth port request
- https://deepwiki.com/google-gemini/gemini-cli/2.2-authentication — DeepWiki on gemini-cli auth

**Azure / M365:**
- https://learn.microsoft.com/en-us/microsoft-copilot-studio/configuration-authentication-azure-ad — Entra ID config
- https://techcommunity.microsoft.com/blog/azure-ai-foundry-blog/deploying-an-agentic-service-to-microsoft-365-copilot-with-delegated-obo-access/4514197 — OBO flow for M365 Copilot agents
- https://learn.microsoft.com/en-us/entra/id-governance/agent-id-governance-overview — Microsoft Agent 365 / Entra Agent ID

**Other providers:**
- https://cursor.com/docs/models-and-pricing — Cursor pricing
- https://github.com/Nomadcxx/opencode-cursor — Cursor proxy (high-churn-risk)
- https://docs.perplexity.ai/docs/getting-started/pricing — Perplexity pricing
- https://goosed.ie/news/perplexity-pro-quietly-removes-free-api-credits/ — Perplexity removes Pro API credits Feb 2026
- https://www.perplexity.ai/api-platform — Perplexity API platform

**Token storage / OSS auth patterns:**
- https://github.com/sst/opencode/issues/4318 — opencode keyring storage feature request
- https://opencode.ai/docs/providers/ — opencode auth methods incl. Claude removal in v1.3.0
- https://opencode.ai/docs/cli/ — opencode CLI
- https://thoughts.jock.pl/p/ai-coding-harness-agents-2026 — 2026 harness comparison

**Compare/landscape:**
- https://www.morphllm.com/ai-coding-assistant-open-source — OSS coding assistants 2026
- https://www.morphllm.com/best-ai-coding-agents-2026 — best agents 2026
- https://www.lumetric.ai/resources/anthropic-just-locked-the-door-on-third-party-harnesses-now-what — post-mortem
- https://www.sovereignmagazine.com/article/anthropic-blocks-openclaw-claude-subscriptions — OpenClaw block coverage
- https://dev.to/mcrolly/anthropic-kills-claude-subscription-access-for-third-party-tools-like-openclaw-what-it-means-for-3ipc — developer-facing analysis
- https://www.cryptika.com/anthropic-officially-ends-claude-subscriptions-for-third-party-tools-like-openclaw/ — Cryptika coverage
- https://www.betterclaw.io/blog/openclaw-anthropic-migration — migration guide post-ban
- https://www.mindstudio.ai/blog/anthropic-openclaw-ban-oauth-authentication — OpenClaw ban explainer
- https://www.mindstudio.ai/blog/openai-codex-anthropic-subscription-change — OpenAI Codex contextual changes
