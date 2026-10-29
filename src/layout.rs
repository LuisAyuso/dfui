use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use taffy::util::ResolveOrZero;
use taffy::{
    AvailableSpace, Dimension, LengthPercentage, NodeId, Size, TaffyTree, compute_leaf_layout,
};

use crate::render::Rect;
use crate::text::Text;
use crate::widget::{Element, State, Widget};

pub(crate) struct Node<'a, Msg> {
    pub w: &'a dyn Widget<Msg>,
    pub id: u64,
    pub parent: Option<usize>,
    /// One past the last descendant in pre-order.
    pub end: usize,
}

/// Taffy's result for a node: box relative to its parent's origin, plus the
/// extent of its children (scrollable content).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(crate) struct Measured {
    pub rect: Rect,
    pub content: Size<f32>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Placed {
    pub rect: Rect,
    pub clip: Rect,
    pub content: Size<f32>,
}

/// Pre-order flattening of the element tree, expanding stateful widgets.
pub(crate) fn flatten<'a, Msg>(
    root: &'a Element<Msg>,
    states: &mut HashMap<u64, State>,
) -> Vec<Node<'a, Msg>> {
    fn go<'a, Msg>(
        e: &'a Element<Msg>,
        parent: Option<usize>,
        slot: u64,
        states: &mut HashMap<u64, State>,
        out: &mut Vec<Node<'a, Msg>>,
    ) {
        let mut h = DefaultHasher::new();
        parent.map(|p| out[p].id).hash(&mut h);
        e.key.is_some().hash(&mut h);
        e.key.unwrap_or(slot).hash(&mut h);
        let id = h.finish();
        e.widget.expand(id, states.entry(id).or_default());
        let i = out.len();
        out.push(Node {
            w: e.widget.as_ref(),
            id,
            parent,
            end: 0,
        });
        for (n, c) in e.widget.children().iter().enumerate() {
            go(c, Some(i), n as u64, states, out);
        }
        out[i].end = out.len();
    }
    let mut out = Vec::new();
    go(root, None, 0, states, &mut out);
    out
}

pub(crate) fn children<Msg>(nodes: &[Node<'_, Msg>], i: usize) -> impl Iterator<Item = usize> {
    let end = nodes[i].end;
    let mut j = i + 1;
    std::iter::from_fn(move || {
        (j < end).then(|| {
            let c = j;
            j = nodes[c].end;
            c
        })
    })
}

/// Insets in logical px: top, right, bottom, left.
pub(crate) type Insets = [f32; 4];

struct Entry {
    node: NodeId,
    measure_key: u64,
    /// Rebuild that last saw this id; older entries are removed.
    seen: u32,
}

/// Taffy tree kept across rebuilds. Nodes are matched by id, and only those
/// whose style, children or measured content changed are marked dirty, so
/// taffy recomputes just their path to the root.
pub(crate) struct Layout {
    tree: TaffyTree<u64>,
    entries: HashMap<u64, Entry>,
    /// Nodes for ids repeated within one tree (duplicate keys); they can't be
    /// matched across rebuilds, so they are rebuilt each time.
    spares: Vec<NodeId>,
    generation: u32,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            tree: TaffyTree::new(),
            entries: HashMap::new(),
            spares: Vec::new(),
            generation: 0,
        }
    }
}

impl Layout {
    /// Flex layout via taffy. The root fills the viewport; `insets` (system
    /// bars) are added to its padding so backgrounds bleed edge to edge while
    /// content stays clear. Only needed when the tree or viewport changes.
    pub(crate) fn measure<Msg>(
        &mut self,
        nodes: &[Node<'_, Msg>],
        text: &Text,
        viewport: (f32, f32),
        insets: Insets,
    ) -> Vec<Measured> {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let tree = &mut self.tree;
        for n in self.spares.drain(..) {
            tree.remove(n).unwrap();
        }
        let mut ids = vec![NodeId::from(0u64); nodes.len()];
        let mut index = HashMap::with_capacity(nodes.len());
        for i in (0..nodes.len()).rev() {
            let mut style = nodes[i].w.style();
            if i == 0 {
                style.size = Size {
                    width: Dimension::length(viewport.0),
                    height: Dimension::length(viewport.1),
                };
                let add = |p: LengthPercentage, inset: f32| {
                    LengthPercentage::length(
                        p.resolve_or_zero(Some(viewport.0), |_, _| 0.0) + inset,
                    )
                };
                let p = style.padding;
                style.padding = taffy::Rect {
                    top: add(p.top, insets[0]),
                    right: add(p.right, insets[1]),
                    bottom: add(p.bottom, insets[2]),
                    left: add(p.left, insets[3]),
                };
            }
            let kids: Vec<NodeId> = children(nodes, i).map(|c| ids[c]).collect();
            let (id, key) = (nodes[i].id, nodes[i].w.measure_key());
            ids[i] = match self.entries.get_mut(&id) {
                Some(e) if e.seen != generation => {
                    e.seen = generation;
                    if *tree.style(e.node).unwrap() != style {
                        tree.set_style(e.node, style).unwrap();
                    }
                    if tree.children(e.node).unwrap() != kids {
                        tree.set_children(e.node, &kids).unwrap();
                    }
                    if e.measure_key != key {
                        e.measure_key = key;
                        tree.mark_dirty(e.node).unwrap();
                    }
                    e.node
                }
                seen => {
                    let node = tree.new_leaf_with_context(style, id).unwrap();
                    tree.set_children(node, &kids).unwrap();
                    if seen.is_some() {
                        self.spares.push(node);
                    } else {
                        self.entries.insert(
                            id,
                            Entry {
                                node,
                                measure_key: key,
                                seen: generation,
                            },
                        );
                    }
                    node
                }
            };
            index.insert(id, i);
        }
        self.entries.retain(|_, e| {
            let live = e.seen == generation;
            if !live {
                tree.remove(e.node).unwrap();
            }
            live
        });
        let avail = Size {
            width: AvailableSpace::Definite(viewport.0),
            height: AvailableSpace::Definite(viewport.1),
        };
        tree.compute_layout_with_measure(ids[0], avail, |input, _, ctx, style| {
            compute_leaf_layout(
                input,
                style,
                |_, _| 0.0,
                |known, available| match ctx {
                    Some(id) => nodes[index[&*id]].w.measure(known, available, text),
                    None => Size::ZERO,
                },
            )
        })
        .unwrap();
        (0..nodes.len())
            .map(|i| {
                let l = tree.layout(ids[i]).unwrap();
                Measured {
                    rect: Rect::new(l.location.x, l.location.y, l.size.width, l.size.height),
                    content: children(nodes, i).fold(Size::ZERO, |s, c| {
                        let cl = tree.layout(ids[c]).unwrap();
                        Size {
                            width: s.width.max(cl.location.x + cl.size.width),
                            height: s.height.max(cl.location.y + cl.size.height),
                        }
                    }),
                }
            })
            .collect()
    }
}

/// Absolute boxes from `measure`'s result, with scroll offsets and clips;
/// cheap enough to run on every redraw.
pub(crate) fn place<Msg>(
    nodes: &[Node<'_, Msg>],
    measured: &[Measured],
    states: &mut HashMap<u64, State>,
) -> Vec<Placed> {
    let full = Rect::new(
        f32::MIN / 4.0,
        f32::MIN / 4.0,
        f32::MAX / 2.0,
        f32::MAX / 2.0,
    );
    let mut placed = vec![Placed::default(); nodes.len()];
    let mut offsets = vec![None; nodes.len()];
    for i in 0..nodes.len() {
        let m = measured[i];
        let (origin, clip) = match nodes[i].parent {
            None => ((0.0, 0.0), full),
            Some(p) => {
                let pr = placed[p].rect;
                let (dx, dy) = offsets[p].unwrap_or((0.0, 0.0));
                let clip = if offsets[p].is_some() {
                    placed[p].clip.intersect(&pr)
                } else {
                    placed[p].clip
                };
                ((pr.x - dx, pr.y - dy), clip)
            }
        };
        placed[i] = Placed {
            rect: Rect::new(origin.0 + m.rect.x, origin.1 + m.rect.y, m.rect.w, m.rect.h),
            clip,
            content: m.content,
        };
        offsets[i] = nodes[i].w.scroll(states.entry(nodes[i].id).or_default());
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{Styled, column, row, spacer, text};

    #[test]
    fn column_padding_gap_grow() {
        let root: Element<()> = column([
            row([]).height(20.0).into(),
            spacer().grow(1.0).into(),
            row([]).height(30.0).into(),
        ])
        .padding(10.0)
        .gap(5.0)
        .into();
        let mut states = HashMap::new();
        let nodes = flatten(&root, &mut states);
        let m = Layout::default().measure(&nodes, &Text::default(), (100.0, 200.0), [0.0; 4]);
        let p = place(&nodes, &m, &mut states);
        assert_eq!(p[0].rect, Rect::new(0.0, 0.0, 100.0, 200.0));
        assert_eq!(p[1].rect, Rect::new(10.0, 10.0, 80.0, 20.0));
        // 200 - 2*10 padding - 20 - 30 - 2*5 gap = 120
        assert_eq!(p[2].rect, Rect::new(10.0, 35.0, 80.0, 120.0));
        assert_eq!(p[3].rect, Rect::new(10.0, 160.0, 80.0, 30.0));
    }

    fn list(items: &[(u64, &str)]) -> Element<()> {
        let rows = items.iter().map(|&(k, s)| {
            let r: Element<()> = row([text(s).into(), spacer().grow(1.0).into()]).into();
            r.key(k)
        });
        column([column(rows).gap(4.0).into(), text("footer").into()])
            .padding(8.0)
            .into()
    }

    #[test]
    fn kept_tree_matches_fresh_layout() {
        let steps: [&[(u64, &str)]; 6] = [
            &[(1, "a"), (2, "bb"), (3, "ccc")],
            &[(1, "a"), (2, "a much longer label"), (3, "ccc")],
            &[(3, "ccc"), (1, "a")],
            &[(3, "ccc"), (4, "new"), (1, "a"), (5, "x")],
            &[(4, "dup"), (4, "dup"), (1, "a")],
            &[],
        ];
        let text = Text::default();
        let mut kept = Layout::default();
        for (n, items) in steps.iter().enumerate() {
            let root = list(items);
            let mut states = HashMap::new();
            let nodes = flatten(&root, &mut states);
            let got = kept.measure(&nodes, &text, (300.0, 400.0), [0.0; 4]);
            let want = Layout::default().measure(&nodes, &text, (300.0, 400.0), [0.0; 4]);
            assert_eq!(got, want, "step {n}");
        }
        assert_eq!(
            kept.tree.total_node_count(),
            kept.entries.len() + kept.spares.len()
        );
    }
}
