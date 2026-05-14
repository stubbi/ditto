# ditto-render

NC-doc renderer for Ditto. Projects bi-temporal NC-graph state into per-entity Markdown pages — the **files-as-memory** slot, with strict structural advantages over Karpathy's LLM Wiki, OpenKB+PageIndex, and Epsilla Semantic Graph.

## Output shape

```
out/
├── Person/
│   ├── alice.md
│   └── bob.md
├── Place/
│   ├── nyc.md
│   └── sf.md
├── index.md
└── .ditto-render.json     # manifest: per-page content hashes
```

Each page has:

- Title (from `properties.name` or `node_id` short form)
- Metadata HTML comment with `node_id`, `node_type`, `tenant_id`, `scope_id`
- Properties JSON block
- **Current outgoing facts** — current edges where this node is `src`
- **Current incoming facts** — current edges where this node is `dst`
- **Historical facts** — superseded / invalidated edges, with their valid-time window
- **Provenance** — episodic event IDs that produced each claim

Cross-references are relative Markdown links. The output is readable as a Logseq/Obsidian vault, by GitHub's renderer, or by any LLM that can read files.

## Why this beats the alternatives

| | Karpathy / OpenKB / Epsilla | ditto-render |
|---|---|---|
| Source of truth | the wiki itself | NC-graph (the wiki is a projection) |
| Time travel | none — `lint` flags stale claims | first-class — historical section renders directly from `t_valid`/`t_invalid` |
| Provenance | depends on synthesis chain | every claim links to episodic event IDs |
| Determinism | LLM synthesis = non-deterministic | deterministic — same graph state → byte-identical Markdown |
| Idempotent re-renders | wiki diffs on every synthesis | content-hashed; no-op when graph unchanged |
| Multi-tenant | single user / shared corpus | per-tenant + per-scope partitions enforced at storage |
| Audit | none | manifest stores per-page content hash; SCITT-signed receipts in NC-graph |

## What it doesn't do (deliberately)

- **No LLM synthesis on render.** This is the v2 architecture's whole point: the graph captures what we know; the renderer is a pure projection. If you want LLM-generated summaries, they live in `Reflective` records that the consolidator writes — those then *render* like any other claim.
- **No editing.** Hand-edits get overwritten. The graph is the source of truth.
- **No vector embeddings.** Following the trending de-emphasis (Supermemory, ByteRover); the renderer's job is structure, not retrieval. Retrieval lives in the controller.

## Filesystem backends

- `LocalFilesystem` — writes under a root directory. Used by `ditto render --out ./brain`.
- `InMemoryFilesystem` — tests and in-process embedding.
- S3 / git-backed / WASM-friendly backends — straightforward `Filesystem` trait impls; forthcoming.
