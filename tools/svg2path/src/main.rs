//! Converts SVG icons into the binary paths `dfui::Shape::from_bytes` reads, so apps bundle
//! native shapes and parse no SVG at runtime.
//!
//! `svg2path <file.svg | dir> <out dir>` writes `<name>.path` for each SVG. usvg resolves the
//! document (shapes, transforms, arcs, viewBox) into paths; strokes become filled outlines, and
//! cubics the quadratics dfui draws.
//!
//! Format, little-endian: `DFP1`, the view box as four f32, a u32 count of contours, a u32 count
//! of quads for each contour, then each quad as six f32.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use usvg::tiny_skia_path::{self as tsp, PathSegment};

type Point = [f32; 2];
type Quad = [Point; 3];

/// Stroke outlines are flattened for drawing up to this many times the SVG's size.
const RESOLUTION: f32 = 4.0;
/// Quadratics per cubic; plenty for icon-sized arcs.
const CUBIC_SPLIT: usize = 4;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, out] = args.as_slice() else {
        return Err("usage: svg2path <file.svg | dir> <out dir>".into());
    };
    fs::create_dir_all(out)?;
    let input = Path::new(input);
    let mut files: Vec<PathBuf> = match input.is_dir() {
        true => fs::read_dir(input)?
            .map(|e| Ok(e?.path()))
            .collect::<Result<_, std::io::Error>>()?,
        false => vec![input.to_path_buf()],
    };
    files.retain(|f| f.extension().is_some_and(|e| e == "svg"));
    files.sort();
    if files.is_empty() {
        return Err(format!("no .svg files in {}", input.display()).into());
    }
    for f in files {
        let bytes = convert(&fs::read(&f)?).map_err(|e| format!("{}: {e}", f.display()))?;
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        fs::write(Path::new(out).join(format!("{name}.path")), &bytes)?;
        println!("{name}: {} bytes", bytes.len());
    }
    Ok(())
}

fn convert(svg: &[u8]) -> Result<Vec<u8>, usvg::Error> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default())?;
    let mut contours = Vec::new();
    collect(tree.root(), &mut contours);
    let s = tree.size();
    let mut out = b"DFP1".to_vec();
    for v in [0.0, 0.0, s.width(), s.height()] {
        out.extend(f32::to_le_bytes(v));
    }
    out.extend((contours.len() as u32).to_le_bytes());
    for c in &contours {
        out.extend((c.len() as u32).to_le_bytes());
    }
    for v in contours.iter().flatten().flatten().flatten() {
        out.extend(v.to_le_bytes());
    }
    Ok(out)
}

fn collect(g: &usvg::Group, out: &mut Vec<Vec<Quad>>) {
    for node in g.children() {
        match node {
            usvg::Node::Group(g) => collect(g, out),
            usvg::Node::Path(p) if p.is_visible() => {
                let ts = p.abs_transform();
                if p.fill().is_some()
                    && let Some(d) = p.data().clone().transform(ts)
                {
                    out.extend(contours(&d));
                }
                if let Some(s) = p.stroke()
                    && let Some(d) = p.data().stroke(&s.to_tiny_skia(), RESOLUTION)
                    && let Some(d) = d.transform(ts)
                {
                    // The stroker turns every outline the same way, so overlapping strokes add
                    // up under dfui's nonzero fill instead of cancelling.
                    out.extend(contours(&d));
                }
            }
            _ => {}
        }
    }
}

/// Closed contours of quadratics.
fn contours(d: &tsp::Path) -> Vec<Vec<Quad>> {
    let (mut out, mut cur) = (Vec::new(), Vec::new());
    let (mut start, mut at): (Point, Point) = ([0.0; 2], [0.0; 2]);
    let close = |cur: &mut Vec<Quad>, at: Point, start: Point, out: &mut Vec<Vec<Quad>>| {
        if at != start {
            cur.push(line(at, start));
        }
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    let pt = |p: tsp::Point| [p.x, p.y];
    for seg in d.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                close(&mut cur, at, start, &mut out);
                (start, at) = (pt(p), pt(p));
            }
            PathSegment::LineTo(p) => {
                cur.push(line(at, pt(p)));
                at = pt(p);
            }
            PathSegment::QuadTo(c, p) => {
                cur.push([at, pt(c), pt(p)]);
                at = pt(p);
            }
            PathSegment::CubicTo(c1, c2, p) => {
                cubic([at, pt(c1), pt(c2), pt(p)], &mut cur);
                at = pt(p);
            }
            PathSegment::Close => {
                close(&mut cur, at, start, &mut out);
                at = start;
            }
        }
    }
    close(&mut cur, at, start, &mut out);
    out
}

fn line(a: Point, b: Point) -> Quad {
    [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b]
}

/// `c` as `CUBIC_SPLIT` quadratics, each matching its piece's ends and tangents' midpoint.
fn cubic(c: [Point; 4], out: &mut Vec<Quad>) {
    let at = |t: f32| {
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        [0, 1].map(|a| (0..4).map(|i| w[i] * c[i][a]).sum::<f32>())
    };
    let tangent = |t: f32| {
        let u = 1.0 - t;
        let w = [
            -3.0 * u * u,
            3.0 * u * u - 6.0 * u * t,
            6.0 * u * t - 3.0 * t * t,
            3.0 * t * t,
        ];
        [0, 1].map(|a| (0..4).map(|i| w[i] * c[i][a]).sum::<f32>())
    };
    let n = CUBIC_SPLIT as f32;
    for i in 0..CUBIC_SPLIT {
        let (t0, t1) = (i as f32 / n, (i + 1) as f32 / n);
        let (p0, p3) = (at(t0), at(t1));
        let (d0, d1) = (tangent(t0), tangent(t1));
        let h = (t1 - t0) / 3.0;
        // The piece's cubic controls, then the quadratic between them.
        let c1 = [p0[0] + d0[0] * h, p0[1] + d0[1] * h];
        let c2 = [p3[0] - d1[0] * h, p3[1] - d1[1] * h];
        let q = [0, 1].map(|a| (3.0 * (c1[a] + c2[a]) - p0[a] - p3[a]) / 4.0);
        out.push([p0, q, p3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Winding number at `p`, counting crossings of a ray toward +x against each quad's
    /// control polygon; close enough for these straight-edged checks.
    fn winding(path: &dfui::path::Path, p: Point) -> i32 {
        let mut w = 0;
        for q in path.contours.iter().flatten() {
            for (a, b) in [(q[0], q[1]), (q[1], q[2])] {
                if (a[1] <= p[1]) != (b[1] <= p[1]) {
                    let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    if x > p[0] {
                        w += if b[1] > a[1] { 1 } else { -1 };
                    }
                }
            }
        }
        w
    }

    #[test]
    fn strokes_become_filled_outlines_that_dfui_reads_back() {
        // An arrow: the shaft drawn downward, the head the other way; they overlap at the tip.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
            stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <line x1="12" y1="2" x2="12" y2="20"/><polyline points="16 16 12 20 8 16"/></svg>"#;
        let s = dfui::Shape::from_bytes(&convert(svg).unwrap()).unwrap();
        let bytes = convert(svg).unwrap();
        assert!(dfui::Shape::from_bytes(&bytes[..bytes.len() - 4]).is_none(), "truncated");
        assert_eq!(s.view(), dfui::Rect::new(0.0, 0.0, 24.0, 24.0));
        assert_ne!(winding(s.path(), [12.0, 10.0]), 0, "on the shaft");
        assert_ne!(
            winding(s.path(), [12.0, 19.5]),
            0,
            "where shaft and head overlap"
        );
        assert_eq!(winding(s.path(), [14.0, 10.0]), 0, "beside the shaft");
    }
}
