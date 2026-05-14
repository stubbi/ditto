# Multi-tenant data model

Ditto is multi-tenant from line one. This document is the contract.

## Why this exists

Every incumbent harness pays single-tenant debt:

- Openclaw's RBAC RFC (#8081, 28 reactions) was **closed as not planned**. The distributed-runtime RFC (#42026) is dormant.
- Hermes is single-user by README. Adding multi-user is a rewrite, not a feature.
- Openhuman is a desktop app. Multi-user isn't a planned axis.
- Gbrain's source isolation leaks at the schema level — pages get attributed to `default` instead of named source (#428, #497, #705 cross-source data loss on delete, #710, #711, #784, #891, #960).

Retrofitting multi-tenancy onto a single-tenant data model is brutal. Building it in costs maybe 20% extra up front and eliminates an entire class of vulnerability later.

## Hierarchy

```
Org              billing entity, SSO root, IdP binding
└── Tenant       isolation boundary, encryption scope, audit scope
    └── Workspace   project / environment unit, RBAC scope
        └── Agent     runtime instance with skills, memory, channels
```

- **Org** owns billing, SSO config, IdP trust. An Org may contain one or many Tenants (e.g., separate Tenants for `prod`, `staging`, `eu`, `us`).
- **Tenant** is the unit of data isolation. Every row in every table that is not strictly global carries `tenant_id`. RLS keys on this column. Encryption keys are derived per Tenant. Audit logs are partitioned per Tenant. Two Tenants in the same Org cannot read each other's data without an explicit federated grant.
- **Workspace** is the RBAC scope. Users hold roles at the Workspace level; admins hold roles at the Tenant level. Workspaces are not isolation boundaries — admins can read across them inside a Tenant — but they are policy boundaries.
- **Agent** is the runtime instance. Skills, memory, channels, cron, and tool grants are all attached at Agent scope. Agents in the same Workspace may share memory under explicit configuration; agents across Workspaces never share memory implicitly.

## Identity

- **Users** authenticate via OIDC. SSO (SAML/OIDC) is configured at the Org. SCIM provisioning is on the roadmap; the initial release supports IdP-driven JIT.
- **Service accounts** are first-class. CI runners, programmatic agents, and external integrations authenticate as service accounts with their own audit identity. Service accounts have a Tenant scope and a Workspace-role grant; they do not have a user identity.
- **Memberships** are (User × Org × Role) and (User × Workspace × Role). A user with no Workspace role inside a Tenant has read-only access to the Tenant's directory of Workspaces — nothing more.

### Roles (initial set)

| Scope     | Roles                                                                |
|-----------|----------------------------------------------------------------------|
| Org       | `owner`, `billing-admin`, `security-admin`                           |
| Tenant    | `tenant-admin`, `auditor` (read-only across all Workspaces + audit)  |
| Workspace | `maintainer`, `operator`, `viewer`                                   |

`maintainer` can mutate skills, agents, and channels. `operator` can run agents and read memory. `viewer` is read-only.

## Storage

### Relational (Postgres)

Single Postgres cluster, RLS-keyed on `tenant_id`.

```sql
ALTER TABLE agents ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON agents
  USING (tenant_id = current_setting('ditto.tenant_id', true)::uuid);
```

The harness binds `ditto.tenant_id` at the start of every connection via a `SET LOCAL` after authentication. Connection pooling is per-Tenant or via a pgbouncer in `transaction` mode with statement-level rebinding.

Per-Tenant Postgres roles for hot Tenants: large customers get a dedicated `tenant_<id>_app` role with a separate password, so a compromised connection string discloses only one Tenant's data.

### Object storage

S3-compatible. Bucket path prefix: `s3://ditto/<tenant_id>/<workspace_id>/...`. Per-Tenant KMS keys for envelope encryption. Object reads are gated by a presigning service that checks `tenant_id` against the requesting identity.

### Vector storage

Vectors live in pgvector (HNSW indexes) by default, with `tenant_id` as a leading filter column and partial indexes per high-traffic Tenant. Alternative: per-Tenant pgvector schemas for Tenants with >10M vectors. The memory architecture doc (forthcoming) will detail the choice between pgvector, DiskANN-on-disk, and external (Qdrant/Turbopuffer/LanceDB) — that decision is driven by recall × latency × cost benchmarks, not by tenancy concerns.

### Audit log

Append-only. Stored in Postgres for hot reads, mirrored to object storage as ndjson partitions per (Tenant, day). Optional hash-chain (Merkle log) for tamper-evidence — opt-in per Tenant.

Audited events (initial set):

- Auth: login, logout, token issuance, SSO failure
- Agent: agent create/update/delete, skill grant/revoke, channel config change
- Tool: every tool call (tool name, argument hash, result hash, principal, latency)
- Memory: every write (slot, content hash, source, principal), every read above sensitivity threshold
- Secret: every read of a secret (broker-mediated; the agent never holds the secret itself)
- Model: every inference call (model id, token counts, cost, principal)

## Secrets

The harness implements a **credential broker**. Agents never hold raw secrets.

1. Tenant admin provisions a secret (e.g., a Slack OAuth token) into the per-Tenant vault. The vault is sealed with the Tenant's KMS key; the harness's signing key is rotated independently.
2. Skills declare required capabilities in their manifest: `{ "needs": ["slack.send_message"] }`. They do not name secrets directly.
3. At tool-call time, the harness looks up the capability binding for the Agent, fetches the secret from the vault under the Agent's audit identity, and either (a) executes the outbound call itself on the agent's behalf, or (b) injects a short-lived scoped token into the tool's environment.
4. The raw secret never enters the agent's context window, never enters the tool process's environment for longer than the call, and never appears in logs.

This eliminates the Hermes #25477 class of issue (`.env` 0664 leaking everything to local users) and the prompt-injection-exfiltrates-credentials class.

## Source / federation

Each Workspace may declare external **sources** (Slack workspace, GitHub org, Gmail account, etc.). Each source has its own OAuth identity and its own audit identity. Memory writes are tagged with `source_id`, not just `tenant_id`/`workspace_id`. This avoids the gbrain pattern where every source's data collapses to `default` (gbrain #428, #705, #891).

A federated read is an explicit operation with its own audit event: `agent A in source X queried source Y, allowed by grant Z`. There is no implicit cross-source retrieval.

## What this model deliberately doesn't do

- **No Tenant nesting.** A Tenant is the isolation boundary. Sub-Tenants are a feature request that creates ambiguity about which level RLS keys on. Use multiple Tenants in an Org instead.
- **No Workspace-level data isolation.** Workspaces are policy boundaries, not isolation boundaries. If a Workspace must be cryptographically isolated, it's a Tenant.
- **No runtime as a top-level dimension.** An agent's runtime (Claude, Codex, local) is a property of the Agent record, not a separate hierarchy. This keeps the URL space and the data model orthogonal — a Tenant is not "the openclaw tenant" or "the hermes tenant"; it's a Tenant with Agents that happen to run on different runtimes.

## Open questions

- pgvector vs. external vector store: defer to memory architecture doc.
- Per-Tenant compute isolation (separate runner nodes per Tenant) vs. shared pool with cgroup quotas: defer to runtime architecture doc.
- Tenant→Tenant federated grants: needed for partner integrations, not initial release.
- Region pinning (EU-only Tenants): needed for GDPR, not initial release.
