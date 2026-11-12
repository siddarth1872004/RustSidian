//! Force-directed layout with Barnes-Hut repulsion. Meant to run on a worker thread: call
//! `step` until it returns `false` (the simulation has cooled), then stop — no idle CPU.

use crate::model::Graph;
use rayon::prelude::*;

/// Mirrors the force sliders in Obsidian's graph view (`graph.json`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Forces {
    pub center_strength: f32,
    pub repel_strength: f32,
    pub link_strength: f32,
    pub link_distance: f32,
}

impl Default for Forces {
    fn default() -> Self {
        Forces { center_strength: 0.52, repel_strength: 10.0, link_strength: 1.0, link_distance: 250.0 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub pos: Vec<[f32; 2]>,
    vel: Vec<[f32; 2]>,
    /// Nodes held in place by the user (dragging or pinned).
    pub pinned: Vec<bool>,
    /// Simulation temperature; decays towards 0.
    pub alpha: f32,
}

const ALPHA_MIN: f32 = 0.002;
const ALPHA_DECAY: f32 = 0.0228;
const VELOCITY_DECAY: f32 = 0.6;
const THETA2: f32 = 0.81;

#[derive(Clone, Copy)]
struct Cell {
    cx: f32,
    cy: f32,
    half: f32,
    mass: f32,
    comx: f32,
    comy: f32,
    /// Child cell indices, or 0 for none. Cell 0 is the root, so 0 is never a child.
    kids: [u32; 4],
    /// Body index for a leaf holding exactly one body.
    body: u32,
}

const NO_BODY: u32 = u32::MAX;

struct Tree {
    cells: Vec<Cell>,
}

impl Tree {
    fn build(pos: &[[f32; 2]]) -> Tree {
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pos {
            minx = minx.min(p[0]);
            maxx = maxx.max(p[0]);
            miny = miny.min(p[1]);
            maxy = maxy.max(p[1]);
        }
        let half = ((maxx - minx).max(maxy - miny) / 2.0).max(1.0) * 1.01;
        let root = Cell {
            cx: (minx + maxx) / 2.0,
            cy: (miny + maxy) / 2.0,
            half,
            mass: 0.0,
            comx: 0.0,
            comy: 0.0,
            kids: [0; 4],
            body: NO_BODY,
        };
        let mut t = Tree { cells: Vec::with_capacity(pos.len() * 2) };
        t.cells.push(root);
        for (i, p) in pos.iter().enumerate() {
            t.insert(0, i as u32, *p, pos, 0);
        }
        t
    }

    fn quadrant(c: &Cell, p: [f32; 2]) -> usize {
        (p[0] >= c.cx) as usize | (((p[1] >= c.cy) as usize) << 1)
    }

    fn child(&mut self, cell: usize, q: usize) -> usize {
        if self.cells[cell].kids[q] != 0 {
            return self.cells[cell].kids[q] as usize;
        }
        let c = self.cells[cell];
        let h = c.half / 2.0;
        let cx = c.cx + if q & 1 == 1 { h } else { -h };
        let cy = c.cy + if q & 2 == 2 { h } else { -h };
        self.cells.push(Cell {
            cx,
            cy,
            half: h,
            mass: 0.0,
            comx: 0.0,
            comy: 0.0,
            kids: [0; 4],
            body: NO_BODY,
        });
        let id = self.cells.len() - 1;
        self.cells[cell].kids[q] = id as u32;
        id
    }

    fn insert(&mut self, mut cell: usize, body: u32, p: [f32; 2], pos: &[[f32; 2]], mut depth: u32) {
        loop {
            let c = &mut self.cells[cell];
            let m = c.mass + 1.0;
            c.comx = (c.comx * c.mass + p[0]) / m;
            c.comy = (c.comy * c.mass + p[1]) / m;
            c.mass = m;
            let is_leaf = c.kids == [0; 4];
            if is_leaf && c.body == NO_BODY && m == 1.0 {
                c.body = body;
                return;
            }
            // Coincident points would recurse forever; let deep cells hold several bodies.
            if depth > 40 {
                return;
            }
            if is_leaf && c.body != NO_BODY {
                let old = c.body;
                c.body = NO_BODY;
                let op = pos[old as usize];
                let q = Self::quadrant(&self.cells[cell], op);
                let ch = self.child(cell, q);
                let k = &mut self.cells[ch];
                k.mass = 1.0;
                k.comx = op[0];
                k.comy = op[1];
                k.body = old;
            }
            let q = Self::quadrant(&self.cells[cell], p);
            cell = self.child(cell, q);
            depth += 1;
        }
    }

    /// Repulsive force on body `i` at `p`.
    fn force(&self, i: u32, p: [f32; 2], strength: f32) -> [f32; 2] {
        let mut f = [0.0f32; 2];
        let mut stack = [0u32; 128];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let c = &self.cells[stack[sp] as usize];
            if c.mass == 0.0 || c.body == i {
                continue;
            }
            let dx = p[0] - c.comx;
            let dy = p[1] - c.comy;
            let d2 = dx * dx + dy * dy;
            let width = c.half * 2.0;
            let leaf = c.kids == [0; 4];
            if leaf || width * width < THETA2 * d2 {
                let d2 = d2.max(1.0);
                // Same law as d3-force's many-body force: magnitude proportional to 1/distance.
                let k = strength * c.mass / d2;
                f[0] += dx * k;
                f[1] += dy * k;
            } else {
                for &k in &c.kids {
                    if k != 0 && sp < stack.len() {
                        stack[sp] = k;
                        sp += 1;
                    }
                }
            }
        }
        f
    }
}

impl Layout {
    /// Deterministic initial placement on a phyllotaxis spiral (no RNG, stable across runs).
    pub fn new(n: usize) -> Layout {
        let pos = (0..n)
            .map(|i| {
                let r = 12.0 * (i as f32 + 0.5).sqrt();
                let a = i as f32 * 2.399_963_1;
                [r * a.cos(), r * a.sin()]
            })
            .collect();
        Layout { pos, vel: vec![[0.0; 2]; n], pinned: vec![false; n], alpha: 1.0 }
    }

    /// Start from cached positions where available.
    pub fn with_positions(n: usize, known: impl Iterator<Item = (usize, [f32; 2])>) -> Layout {
        let mut l = Layout::new(n);
        let mut any = false;
        for (i, p) in known {
            if i < n && p[0].is_finite() && p[1].is_finite() {
                l.pos[i] = p;
                any = true;
            }
        }
        if any {
            l.alpha = 0.3;
        }
        l
    }

    /// Re-heat after a user interaction (drag, force slider change).
    pub fn reheat(&mut self) {
        self.alpha = self.alpha.max(0.5);
    }

    pub fn is_settled(&self) -> bool {
        self.alpha < ALPHA_MIN
    }

    /// Advance one tick. Returns `false` once the layout has settled.
    pub fn step(&mut self, g: &Graph, forces: &Forces) -> bool {
        let n = self.pos.len();
        if n == 0 || self.is_settled() {
            return false;
        }
        let alpha = self.alpha;
        // Repulsion (Barnes-Hut), in parallel.
        let tree = Tree::build(&self.pos);
        let strength = forces.repel_strength * 100.0;
        let pos = &self.pos;
        let rep: Vec<[f32; 2]> =
            (0..n).into_par_iter().map(|i| tree.force(i as u32, pos[i], strength)).collect();
        for (v, f) in self.vel.iter_mut().zip(&rep) {
            v[0] += f[0] * alpha;
            v[1] += f[1] * alpha;
        }
        // Springs along links, weakened for hubs so they do not collapse.
        for &(a, b) in &g.edges {
            let (a, b) = (a as usize, b as usize);
            if a >= n || b >= n || a == b {
                continue;
            }
            let dx = self.pos[b][0] + self.vel[b][0] - self.pos[a][0] - self.vel[a][0];
            let dy = self.pos[b][1] + self.vel[b][1] - self.pos[a][1] - self.vel[a][1];
            let d = (dx * dx + dy * dy).sqrt().max(0.01);
            let (da, db) = (g.nodes[a].degree.max(1) as f32, g.nodes[b].degree.max(1) as f32);
            let k = (d - forces.link_distance * 0.2) / d * alpha * forces.link_strength / da.min(db);
            let bias = da / (da + db);
            self.vel[b][0] -= dx * k * bias;
            self.vel[b][1] -= dy * k * bias;
            self.vel[a][0] += dx * k * (1.0 - bias);
            self.vel[a][1] += dy * k * (1.0 - bias);
        }
        // Gravity towards the origin, then integrate.
        let c = forces.center_strength * 0.1 * alpha;
        for i in 0..n {
            if self.pinned[i] {
                self.vel[i] = [0.0; 2];
                continue;
            }
            let v = &mut self.vel[i];
            v[0] = (v[0] - self.pos[i][0] * c) * VELOCITY_DECAY;
            v[1] = (v[1] - self.pos[i][1] * c) * VELOCITY_DECAY;
            if v[0].is_finite() && v[1].is_finite() {
                self.pos[i][0] += v[0];
                self.pos[i][1] += v[1];
            } else {
                *v = [0.0; 2];
            }
        }
        self.alpha -= self.alpha * ALPHA_DECAY;
        !self.is_settled()
    }

    /// Positions as bytes for the on-disk layout cache.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.pos.iter().flat_map(|p| [p[0].to_le_bytes(), p[1].to_le_bytes()]).flatten().collect()
    }

    pub fn positions_from_bytes(bytes: &[u8]) -> Vec<[f32; 2]> {
        let (chunks, _) = bytes.as_chunks::<8>();
        chunks
            .iter()
            .map(|c| {
                [f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Node, NodeKind};

    fn graph(n: usize, edges: &[(u32, u32)]) -> Graph {
        let mut g = Graph {
            nodes: (0..n)
                .map(|i| Node { kind: NodeKind::Note(i as u32), label: "".into(), degree: 0, group: None })
                .collect(),
            edges: edges.to_vec(),
        };
        for &(a, b) in edges {
            g.nodes[a as usize].degree += 1;
            g.nodes[b as usize].degree += 1;
        }
        g
    }

    fn dist(l: &Layout, a: usize, b: usize) -> f32 {
        ((l.pos[a][0] - l.pos[b][0]).powi(2) + (l.pos[a][1] - l.pos[b][1]).powi(2)).sqrt()
    }

    #[test]
    fn settles_and_linked_nodes_end_up_closer() {
        let g = graph(6, &[(0, 1), (1, 2), (0, 2), (3, 4)]);
        let mut l = Layout::new(6);
        let mut steps = 0;
        while l.step(&g, &Forces::default()) {
            steps += 1;
            assert!(steps < 2000, "must cool down");
        }
        assert!(l.is_settled() && !l.step(&g, &Forces::default()), "a settled layout does no work");
        assert!(l.pos.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
        assert!(dist(&l, 0, 1) < dist(&l, 0, 3), "linked pair closer than unlinked");
        assert!(dist(&l, 3, 4) < dist(&l, 2, 5));
        for i in 0..6 {
            for j in 0..i {
                assert!(dist(&l, i, j) > 1.0, "nodes must not overlap");
            }
        }
    }

    #[test]
    fn pinned_nodes_do_not_move_and_coincident_points_are_safe() {
        let g = graph(4, &[(0, 1)]);
        let mut l = Layout::with_positions(4, (0..4).map(|i| (i, [5.0, 5.0])));
        l.pinned[0] = true;
        l.reheat();
        for _ in 0..50 {
            l.step(&g, &Forces::default());
        }
        assert_eq!(l.pos[0], [5.0, 5.0]);
        assert!(l.pos.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
    }

    #[test]
    fn barnes_hut_matches_brute_force() {
        let l = Layout::new(400);
        let tree = Tree::build(&l.pos);
        let mut worst: f32 = 0.0;
        for i in (0..400).step_by(37) {
            let approx = tree.force(i as u32, l.pos[i], 1000.0);
            let mut exact = [0.0f32; 2];
            let mut total = 0.0f32;
            for j in 0..400 {
                if i == j {
                    continue;
                }
                let (dx, dy) = (l.pos[i][0] - l.pos[j][0], l.pos[i][1] - l.pos[j][1]);
                let d2 = (dx * dx + dy * dy).max(1.0);
                exact[0] += dx * 1000.0 / d2;
                exact[1] += dy * 1000.0 / d2;
                total += 1000.0 / d2.sqrt();
            }
            // Compare against the total pairwise force magnitude: the net force near the
            // centre of a symmetric layout is close to zero and would exaggerate the error.
            let err = ((approx[0] - exact[0]).powi(2) + (approx[1] - exact[1]).powi(2)).sqrt() / total;
            worst = worst.max(err);
        }
        assert!(worst < 0.05, "relative error {worst}");
    }

    #[test]
    fn layout_cache_round_trip() {
        let l = Layout::new(10);
        assert_eq!(Layout::positions_from_bytes(&l.to_bytes()), l.pos);
    }
}
