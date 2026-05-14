//! The render orchestrator. Walks NC-graph state, projects to Markdown,
//! writes via the configured filesystem, emits a manifest.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;

use ditto_core::{Node, NodeId, ScopeId, TenantId};
use ditto_memory::{Storage};

use crate::error::RenderError;
use crate::filesystem::Filesystem;
use crate::manifest::Manifest;
use crate::markdown::{MarkdownRenderer, NodeRef};

pub struct RenderJob<S: Storage, F: Filesystem> {
    storage: Arc<S>,
    fs: Arc<F>,
}

#[derive(Clone, Debug)]
pub struct RenderReport {
    pub pages_written: usize,
    pub pages_unchanged: usize,
    pub pages_removed: usize,
    pub manifest: Manifest,
}

impl<S: Storage, F: Filesystem> RenderJob<S, F> {
    pub fn new(storage: Arc<S>, fs: Arc<F>) -> Self {
        Self { storage, fs }
    }

    /// Render every page for `(tenant_id, scope_id?)`.
    ///
    /// Returns a report describing pages written/unchanged/removed and the
    /// manifest that was persisted to `.ditto-render.json`.
    pub async fn render(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> Result<RenderReport, RenderError> {
        let prior_manifest = self.load_manifest().await?;
        let nodes: Vec<Node> = self.storage.list_nodes(tenant_id, scope_id).await?;
        let node_index: BTreeMap<NodeId, NodeRef> = nodes
            .iter()
            .map(|n| (n.node_id, NodeRef::from_node(n)))
            .collect();

        let renderer = MarkdownRenderer::new(Utc::now());
        let mut new_pages: BTreeMap<String, String> = BTreeMap::new();
        let mut pages_written = 0usize;
        let mut pages_unchanged = 0usize;

        for node in &nodes {
            let outgoing = self
                .storage
                .edges_from_all_time(tenant_id, node.node_id, None)
                .await?;
            let incoming = self
                .storage
                .edges_to_all_time(tenant_id, node.node_id, None)
                .await?;
            let page = renderer.render_page(node, &outgoing, &incoming, &node_index);
            let unchanged = matches!(
                prior_manifest.as_ref().and_then(|m| m.pages.get(&page.path)),
                Some(h) if h == &page.content_hash
            );
            if unchanged {
                pages_unchanged += 1;
            } else {
                self.fs.write(&page.path, &page.bytes).await?;
                pages_written += 1;
            }
            new_pages.insert(page.path.clone(), page.content_hash);
        }

        // Index
        let index_page = renderer.render_index(&nodes);
        let index_unchanged = matches!(
            prior_manifest.as_ref().map(|m| m.index_hash.as_str()),
            Some(h) if h == index_page.content_hash
        );
        if !index_unchanged {
            self.fs.write(&index_page.path, &index_page.bytes).await?;
            pages_written += 1;
        } else {
            pages_unchanged += 1;
        }

        // Remove pages that existed but are no longer projected.
        let mut pages_removed = 0usize;
        if let Some(prior) = &prior_manifest {
            for old_path in prior.pages.keys() {
                if !new_pages.contains_key(old_path) {
                    self.fs.remove(old_path).await?;
                    pages_removed += 1;
                }
            }
        }

        let manifest = Manifest {
            version: 1,
            regenerated_at: renderer.regenerated_at,
            tenant_id,
            scope_id,
            pages: new_pages,
            index_hash: index_page.content_hash,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        self.fs.write(Manifest::PATH, &manifest_bytes).await?;

        Ok(RenderReport {
            pages_written,
            pages_unchanged,
            pages_removed,
            manifest,
        })
    }

    async fn load_manifest(&self) -> Result<Option<Manifest>, RenderError> {
        match self.fs.read(Manifest::PATH).await? {
            Some(bytes) => {
                let m: Manifest = serde_json::from_slice(&bytes)
                    .map_err(|e| RenderError::Manifest(format!("parse: {e}")))?;
                Ok(Some(m))
            }
            None => Ok(None),
        }
    }
}
