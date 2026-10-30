//! The collector: a staging area for things the user intends to remove.
//!
//! Nothing is deleted from here directly; [`crate::delete`] does that, behind a
//! guard. The one subtlety is overlap: collecting a folder and then one of its
//! children must not count the child twice.

use crate::model::{NodeId, Tree};
use std::collections::BTreeSet;

#[derive(Debug, Default, Clone)]
pub struct Collector {
    items: BTreeSet<NodeId>,
}

impl Collector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, id: NodeId) {
        self.items.insert(id);
    }

    pub fn remove(&mut self, id: NodeId) {
        self.items.remove(&id);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.items.contains(&id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.items.iter().copied()
    }

    /// Total space that would be freed, counting each byte once even when an
    /// ancestor and its descendant are both collected.
    #[must_use]
    pub fn total(&self, tree: &Tree) -> u64 {
        self.items
            .iter()
            .filter(|&&id| !self.has_collected_ancestor(tree, id))
            .map(|&id| tree.node(id).total_bytes)
            .sum()
    }

    /// Collected nodes with no collected ancestor, sorted deepest first so a
    /// directory is never removed before its children.
    #[must_use]
    pub fn deletion_order(&self, tree: &Tree) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self
            .items
            .iter()
            .copied()
            .filter(|&id| !self.has_collected_ancestor(tree, id))
            .collect();
        ids.sort_by_key(|&id| std::cmp::Reverse(tree.depth_of(id)));
        ids
    }

    fn has_collected_ancestor(&self, tree: &Tree, id: NodeId) -> bool {
        let mut cursor = tree.node(id).parent;
        while let Some(parent) = cursor {
            if self.items.contains(&parent) {
                return true;
            }
            cursor = tree.node(parent).parent;
        }
        false
    }
}
