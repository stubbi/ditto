# Importer

Ditto is not a hosted version of hermes-agent or openclaw. It imports their state **once** and runs natively. There is no permanent shim, no compatibility runtime, no parallel execution path. The importer is a conversion tool.

## Why one-shot, not a shim

Running a competitor's runtime as a long-term backend means inheriting their bug class forever:

- Openclaw's release treadmill — weekly regressions in channel adapters, gateway stability, MCP plumbing.
- Hermes' 7,261 open PRs and "salvage" merge pattern.
- Gbrain's schema migration breakage every minor version.

If Ditto's value proposition is memory coherence + sandboxing + multi-tenant + eval, none of those work through a shim into a different runtime's data model. The shim becomes the constraint.

The trade-off: migration becomes a one-time conversion event, not a gradual cutover. Users keep their old setup running until they're confident, run `ditto import --dry-run` until the diff is clean, then run `ditto import` and stop the old runtime.

## Provenance contract

Every imported entity carries a provenance record:

```yaml
imported:
  from: hermes-agent | openclaw | openhuman | gbrain | mempalace
  version: <upstream version string>
  imported_at: <ISO 8601>
  source_path: <absolute path on import host>
  source_checksum: <sha256 of the source artifact>
  importer_version: <ditto importer semver>
```

This is queryable: `ditto memory query --provenance from=hermes-agent` returns every memory record sourced from a Hermes import. It is also auditable: an `imported` event is written to the Tenant's audit log for every import operation, with a manifest of what was created.

## hermes-agent importer

### Source layout

Hermes stores everything under `~/.hermes/`:

```
~/.hermes/
├── skills/                  procedural memory (agentskills.io-compatible)
│   └── <skill-name>/SKILL.md
├── MEMORY.md                flat semantic memory (free-form)
├── USER.md                  user profile
├── SOUL.md                  persona / personality config
├── sessions/                FTS5 SQLite store of historical sessions
├── gateway/
│   ├── config.yaml          channel adapter config (Telegram, Slack, …)
│   └── pid                  runtime state (skipped on import)
├── cron/                    cron jobs
├── plugins/                 in-tree and installed plugins
└── providers.yaml           model provider config
```

### Mapping

| Hermes entity                            | Ditto target                                    | Notes |
|------------------------------------------|-------------------------------------------------|-------|
| `skills/<name>/SKILL.md`                 | Skill, scope=Workspace, signed-on-import        | Agentskills.io spec is forward-compatible. Unsigned skills get an `imported:hermes:unverified` tag and require manual approval before re-use. |
| `MEMORY.md`                              | Semantic memory slot, one record per top-level section | The free-form structure is lossy. Importer splits on headings, embeds each section, attaches `imported:hermes:flat-memory` provenance. |
| `USER.md`                                | User profile (Tenant-level), with provenance    | Merged into Tenant's user profile if one exists. |
| `SOUL.md`                                | Agent persona config                            | Persona is per-Agent; importer creates one Agent named `imported-default` and attaches the persona. |
| `sessions/<id>.sqlite`                   | Episodic memory, bulk-imported                  | Each session becomes a batch of episodic records with original timestamps preserved. FTS5 content is re-embedded under Ditto's embedding model. |
| `gateway/config.yaml`                    | Channel configs per Agent                       | Only channels Ditto supports are imported; unsupported channels (initial release: Feishu, WeCom, DingTalk, QQBot) emit a report entry. |
| `cron/`                                  | Routines (Workspace-scope)                      | Cron schedules and prompts translate 1:1. The Hermes-side cron daemon is not used. |
| `plugins/`                               | **Not imported**, reported only                 | In-tree plugins are runtime-specific. The importer emits a list of plugins the user was relying on so they can find or build Ditto equivalents. |
| `providers.yaml`                         | Model routing config                            | OpenAI/Anthropic keys are *not* copied — the user re-authorizes through subscription OAuth or re-enters BYOK keys into the vault. This is intentional. |
| `gateway/pid`, FTS5 lock files, sockets | Skipped                                         | Runtime state. |

### CLI

```
ditto import hermes-agent \
  --source ~/.hermes \
  --tenant <tenant-id> \
  --workspace <workspace-id> \
  [--dry-run]
  [--include sessions,memory,skills,channels,cron]
  [--exclude plugins]
  [--re-embed]  # default: true
```

Output: a per-entity report (`./ditto-import-<timestamp>.json`) listing what was created, skipped, downgraded, or flagged. A summary table prints to stdout.

## openclaw importer

### Source layout

Openclaw is significantly more sprawling. Workspaces live under `~/.openclaw/workspace/` with skills, plugins, MCP configs, sandboxes, sessions, and three concurrent memory subsystems (memory-core, memory-lancedb, memory-wiki).

```
~/.openclaw/
├── workspace/
│   ├── skills/<skill>/SKILL.md
│   ├── memory-core/         flat / structured
│   ├── memory-lancedb/      vector
│   ├── memory-wiki/         markdown wiki
│   ├── sessions/
│   └── plugins/
├── gateway/                 ~22 channel adapters
├── cron/
├── runtimes/                pi-mono, claude-cli, codex, ACPx, opencode-go
├── nodes/                   iOS/Android pairing state
└── plugin-registry/         clawhub install state
```

### Mapping

| Openclaw entity                            | Ditto target                              | Notes |
|--------------------------------------------|-------------------------------------------|-------|
| `workspace/skills/<name>/SKILL.md`         | Skill, signed-on-import                   | Same forward-compat as Hermes. |
| `workspace/memory-core/`                   | Semantic + episodic, split by record type | Lossy. Records without clear structure go to semantic with a `lossy:true` provenance flag. |
| `workspace/memory-lancedb/`                | Re-embedded into Ditto's vector store     | Original embeddings are discarded; Ditto re-embeds under its own model to keep the index consistent. The originals are kept in cold object storage for 30 days in case the user wants a rollback. |
| `workspace/memory-wiki/`                   | Semantic memory with `wiki` provenance    | Wiki pages translate to compiled-truth records (one per page), with append-only event logs migrated as episodic. |
| `workspace/sessions/`                      | Episodic memory                           | Same as Hermes. |
| `workspace/plugins/`, `plugin-registry/`   | **Not imported**, reported only           | Same reason as Hermes. ClawHub plugins are openclaw-runtime-specific. |
| `gateway/<channel>/`                       | Channel configs                           | Initial release imports Slack, Discord, Telegram, WhatsApp, Signal, Matrix, iMessage, Teams. Others emit report entries. |
| `cron/`                                    | Routines                                  | |
| `runtimes/`                                | **Discarded**                             | Ditto runs its own runtime. The user picks a model provider, not a runtime. |
| `nodes/` (iOS/Android pairing)             | Reported, not imported                    | Initial release does not support paired mobile clients. |

### CLI

```
ditto import openclaw \
  --source ~/.openclaw \
  --tenant <tenant-id> \
  --workspace <workspace-id> \
  [--dry-run]
  [--include skills,memory,sessions,channels,cron]
  [--memory-subsystems core,lancedb,wiki]
  [--re-embed]
```

The `--memory-subsystems` flag is openclaw-specific because of the three-subsystem reality. Default is all three, deduplicated.

## Conflict handling

When the import target already has records (e.g., a user importing for the second time, or merging Hermes + Openclaw into one workspace):

- **Skills**: by name. Existing skill with same name + version → skip with conflict report. Same name + different version → side-by-side with version suffix.
- **Memory**: by content hash. Identical hashes → dedupe silently. Near-identical (cosine > 0.97 under Ditto's embedding model) → flag for review queue, don't auto-dedupe.
- **Channels**: by `(channel_type, identifier)`. Conflicts always halt the import with an explicit `--on-conflict overwrite|skip|abort` flag required.
- **Routines / cron**: by name. Conflicts surface in the report; no auto-merge.

## What the importer is not

- **Not bidirectional.** Ditto does not export back to Hermes or Openclaw format. Migration is one-way by design.
- **Not a runtime.** The importer never executes hermes-agent or openclaw code. It reads their on-disk artifacts.
- **Not online.** The importer runs against an on-disk snapshot. A running Hermes or Openclaw instance should be stopped (or at least quiesced) before import to avoid mid-write artifacts.

## Open questions

- gbrain importer: gbrain is a memory layer over openclaw/hermes, not a competing harness. Importing a gbrain palace as a memory source (rather than a competitor cutover) is the right framing; defer to memory architecture doc.
- openhuman importer: openhuman's Composio-OAuth state is the hard part — re-authorizing 118 integrations is a wall. First release likely punts on openhuman import.
- mempalace: same status as gbrain — a memory backend Ditto can read from natively via MCP, not a runtime to migrate off.
