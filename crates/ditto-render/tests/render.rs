//! End-to-end render tests.
//!
//! All tests run against `InMemoryStorage` + `InMemoryFilesystem`. The render
//! logic is storage-impl-agnostic — anything that passes here also passes
//! against `PostgresStorage` once tested with a live database.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ditto_core::{
    EdgeId, InstallKey, NewEdge, NewNode, NodeId, ScopeId, SupersedePolicy, TenantId,
};
use ditto_memory::{InMemoryStorage, MemoryController, Storage};
use ditto_render::{Filesystem, InMemoryFilesystem, Manifest, RenderJob};
use serde_json::json;

fn t(year: i32, month: u32, day: u32) -> chrono::DateTime<chrono::Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

struct Fixture {
    ctrl: MemoryController<InMemoryStorage>,
    fs: Arc<InMemoryFilesystem>,
    storage: Arc<InMemoryStorage>,
    tenant: TenantId,
    scope: ScopeId,
}

impl Fixture {
    fn new() -> Self {
        let storage = Arc::new(InMemoryStorage::new());
        let ctrl =
            MemoryController::new_with_arc(storage.clone(), Arc::new(InstallKey::generate()));
        let fs = Arc::new(InMemoryFilesystem::new());
        Self {
            ctrl,
            fs,
            storage,
            tenant: TenantId::new(),
            scope: ScopeId::new(),
        }
    }

    fn job(&self) -> RenderJob<InMemoryStorage, InMemoryFilesystem> {
        RenderJob::new(self.storage.clone(), self.fs.clone())
    }

    async fn add_person(&self, name: &str) -> NodeId {
        let n = self
            .ctrl
            .assert_node(NewNode {
                node_id: NodeId::new(),
                tenant_id: self.tenant,
                scope_id: self.scope,
                node_type: "Person".into(),
                properties: json!({"name": name}),
                provenance: vec![],
            })
            .await
            .unwrap();
        n.node_id
    }

    async fn add_place(&self, name: &str) -> NodeId {
        let n = self
            .ctrl
            .assert_node(NewNode {
                node_id: NodeId::new(),
                tenant_id: self.tenant,
                scope_id: self.scope,
                node_type: "Place".into(),
                properties: json!({"name": name}),
                provenance: vec![],
            })
            .await
            .unwrap();
        n.node_id
    }

    async fn write_lives_in(
        &self,
        src: NodeId,
        dst: NodeId,
        valid_from: chrono::DateTime<chrono::Utc>,
        supersede: Option<SupersedePolicy>,
    ) {
        self.ctrl
            .write_fact(NewEdge {
                edge_id: EdgeId::new(),
                src,
                dst,
                rel: "lives_in".into(),
                strength: None,
                tenant_id: self.tenant,
                scope_id: self.scope,
                t_valid: valid_from,
                t_invalid: None,
                provenance: vec![],
                supersede,
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn render_writes_a_page_per_node() {
    let f = Fixture::new();
    let alice = f.add_person("Alice").await;
    let _ = f.add_place("Berlin").await;
    f.write_lives_in(alice, _await_berlin(&f).await, t(2026, 1, 1), None)
        .await;

    let report = f.job().render(f.tenant, Some(f.scope)).await.unwrap();
    assert!(report.pages_written >= 2);

    let snap = f.fs.snapshot();
    assert!(snap.contains_key("Person/alice.md"));
    assert!(snap.contains_key("Place/berlin.md"));
    assert!(snap.contains_key("index.md"));
    assert!(snap.contains_key(Manifest::PATH));
}

async fn _await_berlin(f: &Fixture) -> NodeId {
    let nodes = f.storage.list_nodes(f.tenant, Some(f.scope)).await.unwrap();
    nodes
        .iter()
        .find(|n| n.node_type == "Place")
        .map(|n| n.node_id)
        .expect("Berlin missing")
}

#[tokio::test]
async fn alice_page_contains_current_fact() {
    let f = Fixture::new();
    let alice = f.add_person("Alice").await;
    let berlin = f.add_place("Berlin").await;
    f.write_lives_in(alice, berlin, t(2026, 1, 1), None).await;

    f.job().render(f.tenant, Some(f.scope)).await.unwrap();
    let bytes = f.fs.read("Person/alice.md").await.unwrap().unwrap();
    let page = String::from_utf8(bytes).unwrap();

    assert!(page.starts_with("<!-- ditto-render"));
    assert!(page.contains("# Alice"));
    assert!(page.contains("**Type:** `Person`"));
    assert!(page.contains("## Current outgoing facts"));
    assert!(page.contains("**lives_in** → [Berlin](../Place/berlin.md)"));
    assert!(page.contains("since 2026-01-01"));
    assert!(!page.contains("## Historical facts"));
}

#[tokio::test]
async fn bi_temporal_supersession_produces_historical_section() {
    let f = Fixture::new();
    let alice = f.add_person("Alice").await;
    let nyc = f.add_place("NYC").await;
    let sf = f.add_place("SF").await;
    f.write_lives_in(alice, nyc, t(2018, 1, 1), None).await;
    f.write_lives_in(alice, sf, t(2026, 5, 1), Some(SupersedePolicy::AnyWithSameRelation))
        .await;

    f.job().render(f.tenant, Some(f.scope)).await.unwrap();
    let page = String::from_utf8(f.fs.read("Person/alice.md").await.unwrap().unwrap()).unwrap();

    // Current section shows SF; historical shows NYC with its invalidation date.
    assert!(page.contains("## Current outgoing facts"));
    assert!(page.contains("[SF](../Place/sf.md)"));
    assert!(page.contains("since 2026-05-01"));

    assert!(page.contains("## Historical facts"));
    assert!(page.contains("[NYC](../Place/nyc.md)"));
    assert!(page.contains("(2018-01-01 — 2026-05-01)"));
}

#[tokio::test]
async fn idempotent_re_render_produces_no_writes_on_unchanged_graph() {
    let f = Fixture::new();
    let alice = f.add_person("Alice").await;
    let sf = f.add_place("SF").await;
    f.write_lives_in(alice, sf, t(2026, 5, 1), None).await;

    let first = f.job().render(f.tenant, Some(f.scope)).await.unwrap();
    let second = f.job().render(f.tenant, Some(f.scope)).await.unwrap();

    // First run wrote N pages + manifest. Second run should write zero pages
    // (content hashes unchanged); manifest is rewritten unconditionally but
    // is not counted in pages_written.
    assert!(first.pages_written > 0);
    assert_eq!(second.pages_written, 0);
    assert_eq!(second.pages_unchanged, first.pages_written);
}

#[tokio::test]
async fn removing_a_node_removes_its_page_on_next_render() {
    let f = Fixture::new();
    let alice = f.add_person("Alice").await;
    let sf = f.add_place("SF").await;
    f.write_lives_in(alice, sf, t(2026, 5, 1), None).await;

    f.job().render(f.tenant, Some(f.scope)).await.unwrap();
    assert!(f.fs.read("Person/alice.md").await.unwrap().is_some());

    // Wipe tenant — equivalent to "no nodes any more".
    f.ctrl.reset(f.tenant).await.unwrap();
    let report = f.job().render(f.tenant, Some(f.scope)).await.unwrap();

    assert!(report.pages_removed >= 2); // Alice + SF removed
    assert!(f.fs.read("Person/alice.md").await.unwrap().is_none());
    assert!(f.fs.read("Place/sf.md").await.unwrap().is_none());
    let index = String::from_utf8(f.fs.read("index.md").await.unwrap().unwrap()).unwrap();
    assert!(index.contains("_No nodes yet._"));
}

#[tokio::test]
async fn index_md_lists_all_nodes_by_type() {
    let f = Fixture::new();
    f.add_person("Alice").await;
    f.add_person("Bob").await;
    f.add_place("SF").await;

    f.job().render(f.tenant, Some(f.scope)).await.unwrap();
    let index = String::from_utf8(f.fs.read("index.md").await.unwrap().unwrap()).unwrap();

    assert!(index.contains("# Index"));
    assert!(index.contains("## Person (2)"));
    assert!(index.contains("## Place (1)"));
    assert!(index.contains("[Alice](Person/alice.md)"));
    assert!(index.contains("[Bob](Person/bob.md)"));
    assert!(index.contains("[SF](Place/sf.md)"));
}

#[tokio::test]
async fn manifest_records_per_page_content_hashes() {
    let f = Fixture::new();
    f.add_person("Alice").await;
    f.job().render(f.tenant, Some(f.scope)).await.unwrap();

    let manifest_bytes = f.fs.read(Manifest::PATH).await.unwrap().unwrap();
    let m: Manifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(m.version, 1);
    assert!(m.pages.contains_key("Person/alice.md"));
    let hash = &m.pages["Person/alice.md"];
    assert_eq!(hash.len(), 64); // sha256 hex
    assert!(!m.index_hash.is_empty());
}
