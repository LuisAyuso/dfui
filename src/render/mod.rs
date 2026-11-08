//! Drawing: value types (`primitives`), the CPU-side frame (`scene`) and wgpu (`gpu`).

mod gpu;
mod primitives;
mod scene;

pub(crate) use gpu::*;
pub use primitives::*;
pub use scene::*;

#[cfg(test)]
mod tests {
    use super::gpu::scissor;
    use super::*;
    use crate::text::Text;

    #[test]
    fn color_helpers_blend_and_keep_alpha() {
        let c = Color::hex(0x336699).alpha(0.5);
        assert_eq!(c.mix(Color::BLACK, 0.0), c);
        assert_eq!(c.mix(Color::BLACK, 1.0), Color::BLACK);
        assert_eq!(c.lighten(1.0), Color::WHITE.alpha(0.5));
        assert_eq!(c.darken(1.0), Color::BLACK.alpha(0.5));
        assert!((c.darken(0.5).g - c.g / 2.0).abs() < 1e-6);
    }

    fn solid(c: Color) -> Scene {
        let mut s = Scene::default();
        let all = Rect::new(0.0, 0.0, 40.0, 40.0);
        s.shape(all, 0.0, c, 0.0, Color::TRANSPARENT, all);
        s
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn scissored_pass_only_touches_damage() {
        let (red, blue) = (solid(Color::hex(0xff0000)), solid(Color::hex(0x0000ff)));
        let passes = [(&red, None), (&blue, Some([10, 10, 20, 20]))];
        let img = render_offscreen(&passes, &mut Text::default(), (40, 40), 1.0);
        assert_eq!(img.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(img.pixel(15, 15), [0, 0, 255, 255]);
        assert_eq!(img.pixel(35, 35), [255, 0, 0, 255]);
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn image_draws_in_painter_order_between_shapes() {
        // Left half red, right half half-transparent blue.
        let bitmap = Bitmap::from_rgba8(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]);
        let all = Rect::new(0.0, 0.0, 40.0, 40.0);
        let mut s = solid(Color::WHITE);
        s.image(
            Rect::new(0.0, 0.0, 40.0, 20.0),
            &bitmap,
            0.0,
            Color::WHITE,
            all,
        );
        s.shape(
            Rect::new(0.0, 15.0, 40.0, 5.0),
            0.0,
            Color::BLACK,
            0.0,
            Color::TRANSPARENT,
            all,
        );
        let img = render_offscreen(&[(&s, None)], &mut Text::default(), (40, 40), 1.0);
        assert_eq!(img.pixel(2, 5), [255, 0, 0, 255]);
        let [r, g, b, _] = img.pixel(37, 5);
        assert!(
            r.abs_diff(127) <= 2 && g.abs_diff(127) <= 2 && b == 255,
            "blended over white"
        );
        assert_eq!(
            img.pixel(20, 17),
            [0, 0, 0, 255],
            "shape after image covers it"
        );
        assert_eq!(img.pixel(20, 30), [255, 255, 255, 255]);
    }

    #[test]
    fn swapping_bitmaps_damages_the_image() {
        let (a, b) = (
            Bitmap::from_rgba8(1, 1, vec![0; 4]),
            Bitmap::from_rgba8(1, 1, vec![0; 4]),
        );
        let scene = |bm: &Bitmap| {
            let mut s = Scene::default();
            s.image(Rect::new(0.0, 0.0, 10.0, 10.0), bm, 0.0, Color::WHITE, ALL);
            (s, vec![(1, 0..1)])
        };
        let ((sa, na), (sb, nb), (sa2, na2)) = (scene(&a), scene(&b), scene(&a));
        assert!(damage(&sa, &na, &sa2, &na2).is_none());
        assert!(damage(&sa, &na, &sb, &nb).is_some());
    }

    #[test]
    fn scissor_stays_inside_the_target() {
        let size = (800, 600);
        assert_eq!(
            scissor(Rect::new(10.0, 10.0, 20.0, 20.0), 1.0, size),
            [8, 8, 24, 24]
        );
        assert_eq!(
            scissor(Rect::new(790.0, 590.0, 50.0, 50.0), 1.0, size),
            [788, 588, 12, 12]
        );
        // Below the window: nothing to draw, not a 1px strip past the edge.
        let [_, _, w, h] = scissor(Rect::new(23.0, 772.0, 302.0, 202.0), 1.0, size);
        assert_eq!(w * h, 0);
    }

    const ALL: Rect = Rect::new(-1e4, -1e4, 2e4, 2e4);

    /// One square node per `(id, x, color)`, 10px wide, at y = 0.
    fn frame(nodes: &[(u64, f32, u32)]) -> (Scene, Spans) {
        let mut s = Scene::default();
        let mut spans = Vec::new();
        for &(id, x, c) in nodes {
            let start = s.len();
            let r = Rect::new(x, 0.0, 10.0, 10.0);
            s.shape(r, 0.0, Color::hex(c), 0.0, Color::TRANSPARENT, ALL);
            spans.push((id, start..s.len()));
        }
        (s, spans)
    }

    fn diff(a: &[(u64, f32, u32)], b: &[(u64, f32, u32)]) -> Option<Rect> {
        let ((sa, na), (sb, nb)) = (frame(a), frame(b));
        damage(&sa, &na, &sb, &nb)
    }

    #[test]
    fn damage_tracks_changed_nodes_by_id() {
        let base = [(1, 0.0, 0xff0000), (2, 20.0, 0x00ff00)];
        assert_eq!(diff(&base, &base), None);
        // Recolor: just that node, padded 1px for AA.
        let recolored = [(1, 0.0, 0xff0000), (2, 20.0, 0x0000ff)];
        assert_eq!(
            diff(&base, &recolored),
            Some(Rect::new(19.0, -1.0, 12.0, 12.0))
        );
        // Move: old and new position.
        let moved = [(1, 0.0, 0xff0000), (2, 40.0, 0x00ff00)];
        assert_eq!(diff(&base, &moved), Some(Rect::new(19.0, -1.0, 32.0, 12.0)));
        // Insert before node 2 shifts instance indices but not node 2.
        let inserted = [(1, 0.0, 0xff0000), (3, 60.0, 0x0), (2, 20.0, 0x00ff00)];
        assert_eq!(
            diff(&base, &inserted),
            Some(Rect::new(59.0, -1.0, 12.0, 12.0))
        );
        // Removal damages where the node was.
        assert_eq!(
            diff(&base, &base[..1]),
            Some(Rect::new(19.0, -1.0, 12.0, 12.0))
        );
    }

    #[test]
    fn opaque_shape_cuts_a_hole_in_the_shadow_under_it() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let sh = Shadow {
            color: Color::BLACK,
            offset: (0.0, 3.0),
            blur: 8.0,
        };
        let hole = |fill: Color, clip: Rect| {
            let mut s = Scene::default();
            s.shadow(r, 0.0, sh, ALL);
            s.shape(r, 0.0, fill, 0.0, Color::TRANSPARENT, clip);
            s.instances[0].uv
        };
        assert_eq!(hole(Color::WHITE, ALL), [1.0, 1.0, 99.0, 49.0]);
        assert_eq!(
            hole(Color::WHITE, Rect::new(0.0, 0.0, 100.0, 20.0)),
            [1.0, 1.0, 99.0, 20.0]
        );
        // The shadow shows through a translucent shape.
        assert_eq!(hole(Color::WHITE.alpha(0.5), ALL), [0.0; 4]);
    }

    #[test]
    fn damage_bounds_include_blur_and_respect_clip() {
        let mut a = Scene::default();
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        let sh = Shadow {
            color: Color::BLACK,
            offset: (0.0, 0.0),
            blur: 4.0,
        };
        a.shadow(r, 0.0, sh, ALL);
        assert_eq!(a.bounds(0..1), Some(Rect::new(-5.0, -5.0, 20.0, 20.0)));
        let mut b = Scene::default();
        b.shape(
            r,
            0.0,
            Color::BLACK,
            0.0,
            Color::TRANSPARENT,
            Rect::new(0.0, 0.0, 4.0, 4.0),
        );
        assert_eq!(b.bounds(0..1), Some(Rect::new(0.0, 0.0, 4.0, 4.0)));
    }
}
