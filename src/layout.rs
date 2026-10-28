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

/// Flex layout via taffy, then absolute placement with scroll offsets and clips.
/// The root fills the viewport; `insets` (system bars) are added to its padding
/// so backgrounds bleed edge to edge while content stays clear.
pub(crate) fn layout<Msg>(
    nodes: &[Node<'_, Msg>],
    states: &mut HashMap<u64, State>,
    text: &Text,
    viewport: (f32, f32),
    insets: Insets,
) -> Vec<Placed> {
    // ponytail: full relayout every frame; cache taffy nodes if profiling says so.
    let mut tree: TaffyTree<usize> = TaffyTree::new();
    let mut ids = vec![NodeId::from(0u64); nodes.len()];
    for i in (0..nodes.len()).rev() {
        let mut style = nodes[i].w.style();
        if i == 0 {
            style.size = Size {
                width: Dimension::length(viewport.0),
                height: Dimension::length(viewport.1),
            };
            let add = |p: LengthPercentage, inset: f32| {
                LengthPercentage::length(p.resolve_or_zero(Some(viewport.0), |_, _| 0.0) + inset)
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
        ids[i] = if kids.is_empty() {
            tree.new_leaf_with_context(style, i).unwrap()
        } else {
            tree.new_with_children(style, &kids).unwrap()
        };
    }
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
                Some(&mut i) => nodes[i].w.measure(known, available, text),
                None => Size::ZERO,
            },
        )
    })
    .unwrap();

    let full = Rect::new(
        f32::MIN / 4.0,
        f32::MIN / 4.0,
        f32::MAX / 2.0,
        f32::MAX / 2.0,
    );
    let mut placed = vec![Placed::default(); nodes.len()];
    let mut offsets = vec![None; nodes.len()];
    for i in 0..nodes.len() {
        let l = tree.layout(ids[i]).unwrap();
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
        let rect = Rect::new(
            origin.0 + l.location.x,
            origin.1 + l.location.y,
            l.size.width,
            l.size.height,
        );
        let content = children(nodes, i).fold(Size::ZERO, |s, c| {
            let cl = tree.layout(ids[c]).unwrap();
            Size {
                width: s.width.max(cl.location.x + cl.size.width),
                height: s.height.max(cl.location.y + cl.size.height),
            }
        });
        placed[i] = Placed {
            rect,
            clip,
            content,
        };
        offsets[i] = nodes[i].w.scroll(states.entry(nodes[i].id).or_default());
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{Styled, column, row, spacer};

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
        let p = layout(
            &nodes,
            &mut states,
            &Text::default(),
            (100.0, 200.0),
            [0.0; 4],
        );
        assert_eq!(p[0].rect, Rect::new(0.0, 0.0, 100.0, 200.0));
        assert_eq!(p[1].rect, Rect::new(10.0, 10.0, 80.0, 20.0));
        // 200 - 2*10 padding - 20 - 30 - 2*5 gap = 120
        assert_eq!(p[2].rect, Rect::new(10.0, 35.0, 80.0, 120.0));
        assert_eq!(p[3].rect, Rect::new(10.0, 160.0, 80.0, 30.0));
    }
}
