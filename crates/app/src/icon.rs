//! The application icon, drawn procedurally so no image asset or decoder is needed.
//! Shared by the build script (Windows resource) and the app (window icon, welcome screen).

type P = (f32, f32);

/// The crystal outline, clockwise from the top, in unit coordinates.
pub const GEM: [P; 6] = [(0.50, 0.13), (0.75, 0.36), (0.67, 0.79), (0.44, 0.89), (0.25, 0.61), (0.31, 0.30)];
/// Where the facets meet.
pub const GEM_CENTER: P = (0.49, 0.50);
/// Facet colors, one per outline edge, starting with the top-right facet.
pub const FACETS: [[u8; 3]; 6] =
    [[255, 190, 118], [244, 122, 48], [190, 68, 20], [124, 45, 18], [214, 92, 30], [255, 160, 84]];
pub const BG_TOP: [u8; 3] = [52, 38, 78];
pub const BG_BOTTOM: [u8; 3] = [22, 17, 32];

fn in_tri(p: P, a: P, b: P, c: P) -> bool {
    let s = |p: P, q: P, r: P| (p.0 - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.1 - r.1);
    let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// Color at a point of the unit square; alpha 0 outside the rounded background.
fn sample(x: f32, y: f32) -> Option<[u8; 3]> {
    const R: f32 = 0.22;
    let (dx, dy) = ((x - 0.5).abs() - (0.5 - R), (y - 0.5).abs() - (0.5 - R));
    if dx > 0.0 && dy > 0.0 && dx * dx + dy * dy > R * R {
        return None;
    }
    for (i, color) in FACETS.iter().enumerate() {
        if in_tri((x, y), GEM[i], GEM[(i + 1) % GEM.len()], GEM_CENTER) {
            return Some(*color);
        }
    }
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * y) as u8;
    Some([mix(BG_TOP[0], BG_BOTTOM[0]), mix(BG_TOP[1], BG_BOTTOM[1]), mix(BG_TOP[2], BG_BOTTOM[2])])
}

/// The icon as straight (non-premultiplied) RGBA, `size` x `size`, antialiased.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut acc, mut hits) = ([0u32; 3], 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if let Some(c) = sample(x, y) {
                        hits += 1;
                        for (a, v) in acc.iter_mut().zip(c) {
                            *a += u32::from(v);
                        }
                    }
                }
            }
            let n = hits.max(1);
            out.extend([(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8]);
            out.push((hits * 255 / (SS * SS)) as u8);
        }
    }
    out
}
