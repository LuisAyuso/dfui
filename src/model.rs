//! Data behind lists, tables and trees, apart from the widgets that show it.
//! Widgets ask a model how many items there are, for an item and for its
//! key; they rebuild with the rest of the tree and diff rows by key.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Stable identity of an item: rows keep their state and animate by it.
pub trait Keyed {
    fn key(&self) -> u64;
}

/// Items by index, each with a stable key.
pub trait ListModel {
    type Item;
    fn len(&self) -> usize;
    fn with<R>(&self, i: usize, f: impl FnOnce(&Self::Item) -> R) -> R;
    fn key(&self, i: usize) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Keyed> ListModel for Vec<T> {
    type Item = T;
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn with<R>(&self, i: usize, f: impl FnOnce(&T) -> R) -> R {
        f(&self[i])
    }
    fn key(&self, i: usize) -> u64 {
        self[i].key()
    }
}

impl<M: ListModel + ?Sized> ListModel for Rc<M> {
    type Item = M::Item;
    fn len(&self) -> usize {
        (**self).len()
    }
    fn with<R>(&self, i: usize, f: impl FnOnce(&M::Item) -> R) -> R {
        (**self).with(i, f)
    }
    fn key(&self, i: usize) -> u64 {
        (**self).key(i)
    }
}

impl<M: ListModel> ListModel for RefCell<M> {
    type Item = M::Item;
    fn len(&self) -> usize {
        self.borrow().len()
    }
    fn with<R>(&self, i: usize, f: impl FnOnce(&M::Item) -> R) -> R {
        self.borrow().with(i, f)
    }
    fn key(&self, i: usize) -> u64 {
        self.borrow().key(i)
    }
}

/// Items addressed by key, each with ordered children.
pub trait TreeModel {
    type Item;
    /// Calls `f` with the keys of `parent`'s children in order; `None` lists the roots.
    fn children(&self, parent: Option<u64>, f: &mut dyn FnMut(u64));
    fn with<R>(&self, key: u64, f: impl FnOnce(&Self::Item) -> R) -> R;
    /// Lets a node show as expandable before its children are listed.
    fn has_children(&self, key: u64) -> bool {
        let mut any = false;
        self.children(Some(key), &mut |_| any = true);
        any
    }
}

impl<M: TreeModel + ?Sized> TreeModel for Rc<M> {
    type Item = M::Item;
    fn children(&self, parent: Option<u64>, f: &mut dyn FnMut(u64)) {
        (**self).children(parent, f)
    }
    fn with<R>(&self, key: u64, f: impl FnOnce(&M::Item) -> R) -> R {
        (**self).with(key, f)
    }
    fn has_children(&self, key: u64) -> bool {
        (**self).has_children(key)
    }
}

impl<M: TreeModel> TreeModel for RefCell<M> {
    type Item = M::Item;
    fn children(&self, parent: Option<u64>, f: &mut dyn FnMut(u64)) {
        self.borrow().children(parent, f)
    }
    fn with<R>(&self, key: u64, f: impl FnOnce(&M::Item) -> R) -> R {
        self.borrow().with(key, f)
    }
    fn has_children(&self, key: u64) -> bool {
        self.borrow().has_children(key)
    }
}

/// In-memory tree of `Keyed` items.
pub struct Tree<T> {
    roots: Vec<u64>,
    nodes: HashMap<u64, (T, Vec<u64>, Option<u64>)>,
}

impl<T> Default for Tree<T> {
    fn default() -> Self {
        Tree {
            roots: Vec::new(),
            nodes: HashMap::new(),
        }
    }
}

impl<T: Keyed> Tree<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `item` as the last child of `parent`, or as a root.
    pub fn push(&mut self, parent: Option<u64>, item: T) -> u64 {
        let key = item.key();
        match parent.and_then(|p| self.nodes.get_mut(&p)) {
            Some((_, kids, _)) => kids.push(key),
            None => self.roots.push(key),
        }
        self.nodes.insert(key, (item, Vec::new(), parent));
        key
    }

    /// Removes `key` and everything below it.
    pub fn remove(&mut self, key: u64) {
        let Some((_, kids, parent)) = self.nodes.remove(&key) else {
            return;
        };
        let siblings = match parent.and_then(|p| self.nodes.get_mut(&p)) {
            Some((_, s, _)) => s,
            None => &mut self.roots,
        };
        siblings.retain(|&k| k != key);
        for k in kids {
            self.remove(k);
        }
    }

    pub fn get(&self, key: u64) -> Option<&T> {
        self.nodes.get(&key).map(|n| &n.0)
    }
}

impl<T> TreeModel for Tree<T> {
    type Item = T;
    fn children(&self, parent: Option<u64>, f: &mut dyn FnMut(u64)) {
        let kids = match parent {
            None => &self.roots,
            Some(p) => self.nodes.get(&p).map_or(&[][..], |n| &n.1),
        };
        kids.iter().for_each(|&k| f(k));
    }
    fn with<R>(&self, key: u64, f: impl FnOnce(&T) -> R) -> R {
        f(&self.nodes[&key].0)
    }
    fn has_children(&self, key: u64) -> bool {
        self.nodes.get(&key).is_some_and(|n| !n.1.is_empty())
    }
}

/// One visible row of a flattened tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: u64,
    pub depth: u16,
    pub expanded: bool,
    pub has_children: bool,
}

/// The rows a tree shows with the nodes in `open` expanded, depth first.
// ponytail: walks every open node per build; per-node visible counts (GTK's TreeListModel) if huge open trees get slow.
pub fn flatten(model: &impl TreeModel, open: &HashSet<u64>) -> Vec<Row> {
    fn go(
        m: &impl TreeModel,
        parent: Option<u64>,
        depth: u16,
        open: &HashSet<u64>,
        out: &mut Vec<Row>,
    ) {
        let mut kids = Vec::new();
        m.children(parent, &mut |k| kids.push(k));
        for key in kids {
            let expanded = open.contains(&key);
            out.push(Row {
                key,
                depth,
                expanded,
                has_children: m.has_children(key),
            });
            if expanded {
                go(m, Some(key), depth + 1, open, out);
            }
        }
    }
    let mut out = Vec::new();
    go(model, None, 0, open, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct N(u64);
    impl Keyed for N {
        fn key(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn flatten_shows_open_nodes_only() {
        let mut t = Tree::new();
        let a = t.push(None, N(1));
        t.push(Some(a), N(2));
        let c = t.push(Some(a), N(3));
        t.push(Some(c), N(4));
        t.push(None, N(5));
        let keys = |t: &Tree<N>, open: &[u64]| {
            let rows = flatten(t, &open.iter().copied().collect());
            rows.iter().map(|r| (r.key, r.depth)).collect::<Vec<_>>()
        };
        assert_eq!(keys(&t, &[]), [(1, 0), (5, 0)]);
        assert_eq!(keys(&t, &[1]), [(1, 0), (2, 1), (3, 1), (5, 0)]);
        assert_eq!(keys(&t, &[1, 3]), [(1, 0), (2, 1), (3, 1), (4, 2), (5, 0)]);
        assert_eq!(
            keys(&t, &[3]),
            [(1, 0), (5, 0)],
            "closed parent hides open child"
        );
        t.remove(c);
        assert_eq!(keys(&t, &[1, 3]), [(1, 0), (2, 1), (5, 0)]);
        assert!(t.get(4).is_none(), "subtree removed");
    }
}
