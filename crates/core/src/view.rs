//! View pruning.
//!
//! A real home directory holds hundreds of thousands of nodes; a sunburst can only
//! draw a few hundred petals legibly. The view keeps the large children, merges the
//! long tail into one synthetic "smaller objects" petal (exactly as DaisyDisk does),
//! and stops at a ring depth the eye can still follow.

use crate::model::{NodeId, Tree};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ViewNode {
    /// Index of this node inside the returned vector; `children` refer to these.
    pub index: u32,
    /// Index into the source tree, absent for synthetic nodes.
    pub node_id: Option<u64>,
    pub name: String,
    pub bytes: u64,
    pub kind: &'static str,
    pub children: Vec<u32>,
    /// How many source nodes were merged into this one (0 for real nodes).
    pub aggregated_count: u32,
    pub partial: bool,
    /// Gone from disk since the scan; the UI strikes it through.
    pub missing: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewOptions {
    /// Rings to include below the root. Six is already at the edge of legibility.
    pub max_depth: u32,
    /// Hard cap on children per node.
    pub max_children: usize,
    /// Stop keeping siblings once this fraction of the parent is covered.
    pub coverage: f64,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_children: 256,
            coverage: 0.995,
        }
    }
}

/// Build the drawable view rooted at `root`, as a flat vector plus child indices.
#[must_use]
pub fn build_view(tree: &Tree, root: NodeId, opts: &ViewOptions) -> Vec<ViewNode> {
    let mut out: Vec<ViewNode> = Vec::new();
    push_node(tree, root, 0, opts, 0, &mut out);
    out
}

// ─── volume projection (C29) ──────────────────────────────────────────────────────────
//
// Hidden space is computed here, at view time, never stored in the tree. The tree's
// invariant is `total = own + Σ children` over *measured* nodes (C5); the residual is a
// figure that must move when the tree moves (delete, graft, revalidate), so computing it
// from the current tree at every projection makes it self-correcting by construction and
// keeps `Tree` free of a virtual node that deletion, duplicates and classification would
// all have to special-case.

/// The volume's own used space, OS-measured, taken fresh at every projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeAnchor {
    pub used_bytes: u64,
}

/// What [`build_volume_view`] produced beyond the drawable nodes.
#[derive(Debug, Clone)]
pub struct VolumeView {
    pub nodes: Vec<ViewNode>,
    /// `anchor.used_bytes − tree.total_bytes()`, signed and unclamped. Negative means
    /// the tree accounts for more than the volume reports used — a state ("the disk
    /// changed during or after the scan"), for words, not a petal.
    pub residual_bytes: i64,
}

/// The one synthetic node a volume scan adds, drawn after pruning so nothing can merge
/// it away. `kind: "hidden"`, `node_id: None` — the "smaller objects" petal's sibling.
pub const HIDDEN_SPACE_NAME: &str = "Hidden space";

/// As [`build_view`], for the scan root of a volume scan: the root reports the anchor's
/// used bytes (so the header and the ring sum to the volume), and a positive residual
/// draws one synthetic hidden-space child. Callers pass the scan root — the anchor is a
/// volume-level figure and means nothing against a subtree.
///
/// The residual is signed and unclamped in the return. A negative one (the tree totals
/// more than the volume reports used) draws nothing — a zero-width petal would be a
/// lie about the disk — and is the caller's words instead.
#[must_use]
pub fn build_volume_view(
    tree: &Tree,
    root: NodeId,
    opts: &ViewOptions,
    anchor: &VolumeAnchor,
) -> VolumeView {
    let mut nodes = build_view(tree, root, opts);
    // `build_view`'s root children already sum to the subtree's measured total, petal
    // or no petal, so the residual is the one number that makes the ring add up to the
    // volume. i128 because the two endpoints are independent u64 measurements.
    let residual = anchor.used_bytes as i128 - tree.nodes[root].total_bytes as i128;
    let residual_bytes = residual.clamp(i64::MIN as i128, i64::MAX as i128) as i64;

    nodes[0].bytes = anchor.used_bytes;
    if residual > 0 {
        let index = nodes.len() as u32;
        nodes.push(ViewNode {
            index,
            node_id: None,
            name: HIDDEN_SPACE_NAME.to_string(),
            bytes: u64::try_from(residual).unwrap_or(u64::MAX),
            kind: "hidden",
            children: Vec::new(),
            aggregated_count: 0,
            partial: false,
            missing: false,
        });
        nodes[0].children.push(index);
    }

    VolumeView {
        nodes,
        residual_bytes,
    }
}

/// A deleted node is worth nothing, which would make it invisible. Give it a sliver wide
/// enough to see and strike through, so the user can tell what they just did.
const MISSING_FLOOR_DIVISOR: u64 = 200;

fn push_node(
    tree: &Tree,
    id: NodeId,
    depth: u32,
    opts: &ViewOptions,
    floor: u64,
    out: &mut Vec<ViewNode>,
) -> u32 {
    let node = tree.node(id);
    let index = out.len() as u32;
    let bytes = if node.missing {
        node.total_bytes.max(floor)
    } else {
        node.total_bytes
    };
    out.push(ViewNode {
        index,
        node_id: Some(id as u64),
        name: node.name.clone(),
        bytes,
        kind: node.kind.as_str(),
        children: Vec::new(),
        aggregated_count: 0,
        partial: node.partial,
        missing: node.missing,
    });

    if depth >= opts.max_depth || node.children.is_empty() {
        return index;
    }

    let mut kids: Vec<NodeId> = node.children.clone();
    kids.sort_by(|a, b| {
        tree.node(*b)
            .total_bytes
            .cmp(&tree.node(*a).total_bytes)
            .then_with(|| tree.node(*a).name.cmp(&tree.node(*b).name))
    });

    let parent_bytes = node.total_bytes.max(1);
    let child_floor = (parent_bytes / MISSING_FLOOR_DIVISOR).max(1);
    let mut kept: Vec<NodeId> = Vec::new();
    let mut covered: u64 = 0;

    for &kid in &kids {
        let enough = covered as f64 / parent_bytes as f64 >= opts.coverage;
        let room = kept.len() < opts.max_children;
        if !room || (enough && kept.len() >= 8) {
            break;
        }
        kept.push(kid);
        covered = covered.saturating_add(tree.node(kid).total_bytes);
    }

    let mut child_indices: Vec<u32> = kept
        .iter()
        .map(|&kid| push_node(tree, kid, depth + 1, opts, child_floor, out))
        .collect();

    let aggregated = kids.len().saturating_sub(kept.len());
    if aggregated > 0 {
        let rest: u64 = kids[kids.len() - aggregated..]
            .iter()
            .map(|&kid| tree.node(kid).total_bytes)
            .sum();
        if rest > 0 || aggregated > 0 {
            let agg_index = out.len() as u32;
            out.push(ViewNode {
                index: agg_index,
                node_id: None,
                name: "smaller objects".to_string(),
                bytes: rest,
                kind: "aggregate",
                children: Vec::new(),
                aggregated_count: aggregated as u32,
                partial: false,
                missing: false,
            });
            child_indices.push(agg_index);
        }
    }

    out[index as usize].children = child_indices;
    index
}
