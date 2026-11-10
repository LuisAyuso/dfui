use std::collections::{HashMap, HashSet};
use std::f32::consts::FRAC_1_SQRT_2;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};

use super::*;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Instance {
    pub(crate) rect: [f32; 4],
    pub(super) uv: [f32; 4],
    color: [f32; 4],
    border_color: [f32; 4],
    params: [f32; 4],
    clip: [f32; 4],
}

/// Flat list of distance-field primitives for one frame, in logical pixels.
#[derive(Default)]
pub struct Scene {
    pub(crate) instances: Vec<Instance>,
    /// Instance index of each image, in order, with the bitmap it samples.
    pub(super) bitmaps: Vec<(usize, Bitmap)>,
}

pub(crate) enum Outcome {
    /// Drawn and presented; how long acquiring the frame blocked (vsync).
    Presented(std::time::Duration),
    /// Nothing changed, so nothing was presented.
    Idle,
    /// No frame could be acquired; the canvas is untouched.
    Failed,
}

/// Which instances each node painted, by node id.
pub(crate) type Spans = Vec<(u64, Range<usize>)>;

impl Scene {
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Pixels the instances can touch: quad plus AA/blur padding, clipped.
    pub(super) fn bounds(&self, r: Range<usize>) -> Option<Rect> {
        self.instances[r]
            .iter()
            .map(|i| {
                let [x, y, w, h] = i.rect;
                let glyph = i.params[3] > 0.5;
                let pad = if glyph { 1.0 } else { i.params[2] + 1.0 };
                let [cx, cy, cw, ch] = i.clip;
                Rect::new(x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad)
                    .intersect(&Rect::new(cx, cy, cw, ch))
            })
            .filter(|r| r.w > 0.0 && r.h > 0.0)
            .reduce(|a, b| a.union(&b))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn shape(
        &mut self,
        r: Rect,
        radius: f32,
        fill: Color,
        border: f32,
        border_color: Color,
        clip: Rect,
    ) {
        // An opaque shape right over a shadow (a card on its own shadow)
        // hides what's under it; the shadow is drawn as a ring around that.
        let opaque = fill.a >= 1.0 && (border <= 0.0 || border_color.a >= 1.0);
        if opaque
            && let Some(prev) = self.instances.last_mut()
            && prev.params[3] == 0.0
            && prev.params[2] > 0.0
        {
            // Inscribed in the rounded corners, minus 1px of edge antialiasing.
            let k = radius.min(r.w.min(r.h) * 0.5) * (1.0 - FRAC_1_SQRT_2) + 1.0;
            prev.uv = [
                (r.x + k).max(clip.x),
                (r.y + k).max(clip.y),
                (r.x + r.w - k).min(clip.x + clip.w),
                (r.y + r.h - k).min(clip.y + clip.h),
            ];
        }
        self.instances.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            uv: [0.0; 4],
            color: fill.rgba(),
            border_color: border_color.rgba(),
            params: [radius, border, 0.0, 0.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }

    pub fn shadow(&mut self, r: Rect, radius: f32, s: Shadow, clip: Rect) {
        self.instances.push(Instance {
            rect: [r.x + s.offset.0, r.y + s.offset.1, r.w, r.h],
            uv: [0.0; 4],
            color: s.color.rgba(),
            border_color: [0.0; 4],
            params: [radius, 0.0, s.blur.max(0.01), 0.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }

    pub(crate) fn glyph(
        &mut self,
        r: Rect,
        em: [f32; 4],
        color: Color,
        clip: Rect,
        bands: [f32; 3],
        mask: Option<[f32; 3]>,
    ) {
        self.instances.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            uv: em,
            color: color.rgba(),
            border_color: mask.map_or([0.0; 4], |m| [m[0], m[1], m[2], 1.0]),
            params: [bands[0], bands[1], bands[2], 1.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }

    /// `bitmap` stretched over `r`, with rounded corners, multiplied by `tint`.
    pub fn image(&mut self, r: Rect, bitmap: &Bitmap, radius: f32, tint: Color, clip: Rect) {
        self.bitmaps.push((self.instances.len(), bitmap.clone()));
        self.instances.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: tint.rgba(),
            // Only for the damage diff: a different bitmap means different bytes.
            border_color: [f32::from_bits(bitmap.0.id), 0.0, 0.0, 0.0],
            params: [radius, 0.0, 0.0, 2.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }
}

/// Area that differs between two painted frames, or `None` if nothing does.
/// Nodes are matched by id, so inserting one doesn't damage its later siblings.
// ponytail: a pure z-order swap with identical instances isn't detected; hash sibling order if that ever matters.
pub(crate) fn damage(
    old: &Scene,
    old_spans: &Spans,
    new: &Scene,
    new_spans: &Spans,
) -> Option<Rect> {
    fn bytes<'a>(s: &'a Scene, r: &Range<usize>) -> &'a [u8] {
        bytemuck::cast_slice(&s.instances[r.clone()])
    }
    let prev: HashMap<u64, &Range<usize>> = old_spans.iter().map(|(id, r)| (*id, r)).collect();
    let mut seen = HashSet::new();
    let mut out: Option<Rect> = None;
    let mut add = |r: Option<Rect>| {
        if let Some(r) = r {
            out = Some(out.map_or(r, |o| o.union(&r)));
        }
    };
    // Instances before the first span (the background) belong to no node.
    let lead = |s: &Scene, sp: &Spans| 0..sp.first().map_or(s.len(), |(_, r)| r.start);
    let (lo, ln) = (lead(old, old_spans), lead(new, new_spans));
    if bytes(old, &lo) != bytes(new, &ln) {
        add(old.bounds(lo));
        add(new.bounds(ln));
    }
    for (id, r) in new_spans {
        seen.insert(*id);
        match prev.get(id) {
            Some(o) if bytes(old, o) == bytes(new, r) => {}
            Some(o) => {
                add(old.bounds((*o).clone()));
                add(new.bounds(r.clone()));
            }
            None => add(new.bounds(r.clone())),
        }
    }
    for (id, r) in old_spans {
        if !seen.contains(id) {
            add(old.bounds(r.clone()));
        }
    }
    out
}
