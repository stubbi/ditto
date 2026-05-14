//! Markdown rendering: NC-graph state → opinionated Markdown pages.
//!
//! Output shape:
//!
//! - One page per node, at `{node_type}/{slug}.md`.
//! - `index.md` at the root with a by-type catalog.
//! - Each page has: title, metadata comment, properties block, current
//!   outgoing facts, current incoming facts, historical facts, provenance.
//! - Cross-references are relative `.md` links resolvable by any editor that
//!   speaks Markdown (Obsidian, Logseq, VS Code, GitHub).
//!
//! Deterministic. Same NC-graph state → byte-identical output, regardless
//! of insertion order. The `regenerated_at` line is excluded from the
//! content hash so timestamps don't churn manifests.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use ditto_core::{Edge, Node, NodeId};

/// A rendered page: the bytes to write + a stable content hash (excluding
/// the regenerated-at timestamp).
pub struct Page {
    pub path: String,
    pub bytes: Vec<u8>,
    pub content_hash: String,
}

pub struct MarkdownRenderer {
    pub regenerated_at: DateTime<Utc>,
}

impl MarkdownRenderer {
    pub fn new(regenerated_at: DateTime<Utc>) -> Self {
        Self { regenerated_at }
    }

    /// Render a single entity page.
    ///
    /// `outgoing` and `incoming` MUST include both current and historical
    /// edges (the renderer filters them). `node_index` resolves cross-refs
    /// to other entities' file paths.
    pub fn render_page(
        &self,
        node: &Node,
        outgoing: &[Edge],
        incoming: &[Edge],
        node_index: &BTreeMap<NodeId, NodeRef>,
    ) -> Page {
        let path = page_path(node);
        let body = self.render_body(node, outgoing, incoming, node_index, &path);
        let content_hash = hash_content(&body);
        let bytes = self.wrap_with_metadata(&body, &content_hash);
        Page {
            path,
            bytes: bytes.into_bytes(),
            content_hash,
        }
    }

    /// Render the index.md catalog.
    pub fn render_index(&self, nodes: &[Node]) -> Page {
        let body = self.render_index_body(nodes);
        let content_hash = hash_content(&body);
        let bytes = self.wrap_with_metadata(&body, &content_hash);
        Page {
            path: "index.md".to_string(),
            bytes: bytes.into_bytes(),
            content_hash,
        }
    }

    fn render_body(
        &self,
        node: &Node,
        outgoing: &[Edge],
        incoming: &[Edge],
        node_index: &BTreeMap<NodeId, NodeRef>,
        self_path: &str,
    ) -> String {
        let mut out = String::with_capacity(2048);
        let title = node_title(node);

        writeln!(out, "# {title}").unwrap();
        writeln!(out).unwrap();
        writeln!(
            out,
            "<!-- ditto-node: id={} type={} tenant={} scope={} -->",
            node.node_id, node.node_type, node.tenant_id, node.scope_id
        )
        .unwrap();
        writeln!(out).unwrap();
        writeln!(out, "**Type:** `{}`", node.node_type).unwrap();
        writeln!(out).unwrap();

        // Properties section
        if !node.properties.is_null()
            && node.properties.as_object().is_none_or(|o| !o.is_empty())
        {
            writeln!(out, "## Properties").unwrap();
            writeln!(out).unwrap();
            writeln!(out, "```json").unwrap();
            let mut pretty = serde_json::to_string_pretty(&node.properties).unwrap();
            if !pretty.ends_with('\n') {
                pretty.push('\n');
            }
            out.push_str(&pretty);
            writeln!(out, "```").unwrap();
            writeln!(out).unwrap();
        }

        let depth = self_path.matches('/').count();

        // Current outgoing
        let current_out: Vec<&Edge> = outgoing.iter().filter(|e| e.is_current()).collect();
        if !current_out.is_empty() {
            writeln!(out, "## Current outgoing facts").unwrap();
            writeln!(out).unwrap();
            for edge in &current_out {
                let dst_link = link_for(node_index, edge.dst, depth);
                writeln!(
                    out,
                    "- **{}** → {} — since {}",
                    edge.rel,
                    dst_link,
                    fmt_ts(edge.t_valid)
                )
                .unwrap();
            }
            writeln!(out).unwrap();
        }

        // Current incoming
        let current_in: Vec<&Edge> = incoming.iter().filter(|e| e.is_current()).collect();
        if !current_in.is_empty() {
            writeln!(out, "## Current incoming facts").unwrap();
            writeln!(out).unwrap();
            for edge in &current_in {
                let src_link = link_for(node_index, edge.src, depth);
                writeln!(
                    out,
                    "- **{}** ← {} — since {}",
                    edge.rel,
                    src_link,
                    fmt_ts(edge.t_valid)
                )
                .unwrap();
            }
            writeln!(out).unwrap();
        }

        // Historical (non-current outgoing + non-current incoming)
        let historical: Vec<(&str, &Edge)> = outgoing
            .iter()
            .filter(|e| !e.is_current())
            .map(|e| ("→", e))
            .chain(
                incoming
                    .iter()
                    .filter(|e| !e.is_current())
                    .map(|e| ("←", e)),
            )
            .collect();
        if !historical.is_empty() {
            writeln!(out, "## Historical facts").unwrap();
            writeln!(out).unwrap();
            for (arrow, edge) in &historical {
                let other = if *arrow == "→" { edge.dst } else { edge.src };
                let other_link = link_for(node_index, other, depth);
                let end = edge
                    .t_invalid
                    .map(fmt_ts)
                    .unwrap_or_else(|| "current".to_string());
                writeln!(
                    out,
                    "- **{}** {} {} ({} — {})",
                    edge.rel,
                    arrow,
                    other_link,
                    fmt_ts(edge.t_valid),
                    end
                )
                .unwrap();
            }
            writeln!(out).unwrap();
        }

        // Provenance: union of all event_ids referenced
        let mut events: BTreeSet<String> = BTreeSet::new();
        for e in outgoing.iter().chain(incoming.iter()) {
            for ev in &e.provenance {
                events.insert(ev.to_hex());
            }
        }
        for ev in &node.provenance {
            events.insert(ev.to_hex());
        }
        if !events.is_empty() {
            writeln!(out, "## Provenance").unwrap();
            writeln!(out).unwrap();
            writeln!(
                out,
                "Facts about this entity trace to {} episodic event(s):",
                events.len()
            )
            .unwrap();
            writeln!(out).unwrap();
            for ev in &events {
                writeln!(out, "- `{}`", &ev[..16.min(ev.len())]).unwrap();
            }
            writeln!(out).unwrap();
        }

        writeln!(
            out,
            "<!-- This file is a projection of NC-graph state. Do not edit by hand; -->"
        )
        .unwrap();
        writeln!(
            out,
            "<!-- changes are overwritten on the next ditto render. -->"
        )
        .unwrap();

        out
    }

    fn render_index_body(&self, nodes: &[Node]) -> String {
        let mut by_type: BTreeMap<&str, Vec<&Node>> = BTreeMap::new();
        for n in nodes {
            by_type.entry(n.node_type.as_str()).or_default().push(n);
        }

        let mut out = String::with_capacity(1024);
        writeln!(out, "# Index").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "Catalog of NC-doc pages projected from NC-graph state.").unwrap();
        writeln!(out).unwrap();

        if nodes.is_empty() {
            writeln!(out, "_No nodes yet._").unwrap();
            writeln!(out).unwrap();
            return out;
        }

        for (ty, ns) in &by_type {
            writeln!(out, "## {} ({})", ty, ns.len()).unwrap();
            writeln!(out).unwrap();
            let mut sorted: Vec<&&Node> = ns.iter().collect();
            sorted.sort_by_key(|n| n.node_id.0);
            for n in sorted {
                let path = page_path(n);
                let title = node_title(n);
                writeln!(
                    out,
                    "- [{}]({}) — `{}`",
                    title,
                    path,
                    &n.node_id.to_string()[..8]
                )
                .unwrap();
            }
            writeln!(out).unwrap();
        }
        out
    }

    fn wrap_with_metadata(&self, body: &str, content_hash: &str) -> String {
        // The `regenerated_at` line is excluded from the content hash so
        // re-renders with identical graph state don't churn the manifest.
        let mut out = String::with_capacity(body.len() + 256);
        writeln!(out, "<!-- ditto-render: v=1 content_hash={content_hash} -->").unwrap();
        writeln!(
            out,
            "<!-- regenerated-at: {} -->",
            self.regenerated_at.to_rfc3339()
        )
        .unwrap();
        out.push_str(body);
        out
    }
}

/// Compact reference to another node, used for cross-link rendering.
#[derive(Clone)]
pub struct NodeRef {
    pub node_type: String,
    pub slug: String,
    pub title: String,
}

impl NodeRef {
    pub fn from_node(node: &Node) -> Self {
        Self {
            node_type: node.node_type.clone(),
            slug: slug_for(node),
            title: node_title(node),
        }
    }

    fn path(&self) -> String {
        format!("{}/{}.md", sanitize_path_segment(&self.node_type), self.slug)
    }
}

pub fn page_path(node: &Node) -> String {
    format!(
        "{}/{}.md",
        sanitize_path_segment(&node.node_type),
        slug_for(node)
    )
}

pub fn slug_for(node: &Node) -> String {
    let name = node
        .properties
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    match name {
        Some(s) => sanitize_slug(s),
        None => node.node_id.to_string()[..8].to_string(),
    }
}

pub fn node_title(node: &Node) -> String {
    node.properties
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| node.node_id.to_string())
}

fn sanitize_slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = false;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

fn sanitize_path_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        out.push_str("Unknown");
    }
    out
}

fn link_for(index: &BTreeMap<NodeId, NodeRef>, target: NodeId, depth: usize) -> String {
    let nref = match index.get(&target) {
        Some(n) => n,
        None => return format!("`{}` (unresolved)", target),
    };
    let relative = if depth == 0 {
        nref.path()
    } else {
        // For a page at "Person/alice.md" (depth=1), link to a sibling page
        // is "../Place/sf.md".
        let prefix = "../".repeat(depth);
        format!("{}{}", prefix, nref.path())
    };
    format!("[{}]({})", nref.title, relative)
}

fn fmt_ts(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d").to_string()
}

fn hash_content(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_sanitizes_spaces_and_punctuation() {
        assert_eq!(sanitize_slug("Hello World!"), "hello-world");
        assert_eq!(sanitize_slug("Acme, Inc."), "acme-inc");
        assert_eq!(sanitize_slug("---"), "unnamed");
        assert_eq!(sanitize_slug(""), "unnamed");
        assert_eq!(sanitize_slug("Über/Föö"), "ber-f");
    }
}
