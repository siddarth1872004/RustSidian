use crate::{Diagram, Error, Fill, Item, Measure, TEXT_SIZE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Rect,
    Round,
    Stadium,
    Circle,
    Diamond,
}

struct Node {
    id: String,
    label: String,
    shape: Shape,
    w: f32,
    h: f32,
    dummy: bool,
}

struct Edge {
    from: usize,
    to: usize,
    label: String,
    dashed: bool,
    thick: bool,
    head_to: bool,
    head_from: bool,
}

#[derive(Default)]
struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    groups: Vec<(String, Vec<usize>)>,
    open: Vec<usize>,
}

impl Graph {
    fn node(&mut self, id: &str, label: Option<String>, shape: Option<Shape>) -> usize {
        let i = match self.nodes.iter().position(|n| n.id == id) {
            Some(i) => i,
            None => {
                self.nodes.push(Node {
                    id: id.to_string(),
                    label: id.to_string(),
                    shape: Shape::Rect,
                    w: 0.0,
                    h: 0.0,
                    dummy: false,
                });
                self.nodes.len() - 1
            }
        };
        if let Some(l) = label {
            self.nodes[i].label = l;
        }
        if let Some(s) = shape {
            self.nodes[i].shape = s;
        }
        for &g in &self.open {
            if !self.groups[g].1.contains(&i) {
                self.groups[g].1.push(i);
            }
        }
        i
    }
}

struct Cur<'a> {
    s: &'a [char],
    i: usize,
}

impl Cur<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }
    fn starts(&self, p: &str) -> bool {
        p.chars().enumerate().all(|(k, c)| self.s.get(self.i + k) == Some(&c))
    }
    fn rest(&self) -> String {
        self.s[self.i.min(self.s.len())..].iter().collect()
    }
    fn done(&self) -> bool {
        self.i >= self.s.len()
    }
}

fn clean_label(t: &str) -> String {
    let t = t.trim().trim_matches('"').trim();
    t.replace("<br/>", "\n").replace("<br>", "\n").replace("<br />", "\n").replace("\\n", "\n")
}

fn parse_node(c: &mut Cur, g: &mut Graph) -> Result<usize, Error> {
    c.ws();
    let s = c.i;
    while c.s.get(c.i).is_some_and(|ch| ch.is_alphanumeric() || *ch == '_') {
        c.i += 1;
    }
    if c.i == s {
        return Err(Error::Syntax(format!("expected a node near `{}`", c.rest())));
    }
    let id: String = c.s[s..c.i].iter().collect();
    const OPENERS: &[(&str, &[&str], Shape)] = &[
        ("((", &["))"], Shape::Circle),
        ("([", &["])"], Shape::Stadium),
        ("[(", &[")]"], Shape::Rect),
        ("[[", &["]]"], Shape::Rect),
        ("[/", &["/]", "\\]"], Shape::Rect),
        ("[\\", &["\\]", "/]"], Shape::Rect),
        ("{{", &["}}"], Shape::Round),
        ("(", &[")"], Shape::Round),
        ("[", &["]"], Shape::Rect),
        ("{", &["}"], Shape::Diamond),
        (">", &["]"], Shape::Rect),
    ];
    for (open, closers, shape) in OPENERS {
        if c.starts(open) {
            c.i += open.chars().count();
            let quoted = c.s.get(c.i) == Some(&'"');
            let mut text_end = None;
            if quoted {
                if let Some(q) = c.s[c.i + 1..].iter().position(|&x| x == '"') {
                    c.i += 1;
                    let label: String = c.s[c.i..c.i + q].iter().collect();
                    c.i += q + 1;
                    for cl in *closers {
                        if c.starts(cl) {
                            c.i += cl.chars().count();
                            return Ok(g.node(&id, Some(clean_label(&label)), Some(*shape)));
                        }
                    }
                    return Err(Error::Syntax("unclosed node".into()));
                }
            }
            for k in c.i..c.s.len() {
                let hit = closers
                    .iter()
                    .find(|cl| cl.chars().enumerate().all(|(j, ch)| c.s.get(k + j) == Some(&ch)));
                if let Some(cl) = hit {
                    text_end = Some((k, cl.chars().count()));
                    break;
                }
            }
            let Some((k, n)) = text_end else { return Err(Error::Syntax("unclosed node".into())) };
            let label: String = c.s[c.i..k].iter().collect();
            c.i = k + n;
            return Ok(g.node(&id, Some(clean_label(&label)), Some(*shape)));
        }
    }
    Ok(g.node(&id, None, None))
}

struct Op {
    label: String,
    dashed: bool,
    thick: bool,
    head_to: bool,
    head_from: bool,
}

fn run_op(c: &mut Cur) -> Option<(String, bool)> {
    let s = c.i;
    while c.s.get(c.i).is_some_and(|ch| matches!(ch, '-' | '=' | '.' | '<' | '>' | '~')) {
        c.i += 1;
    }
    let mut cross = false;
    if c.i > s
        && matches!(c.s.get(c.i), Some('o' | 'x'))
        && c.s.get(c.i + 1).is_none_or(|ch| ch.is_whitespace())
    {
        c.i += 1;
        cross = true;
    }
    (c.i - s >= 2).then(|| (c.s[s..c.i].iter().collect(), cross))
}

fn parse_op(c: &mut Cur) -> Result<Op, Error> {
    c.ws();
    let Some((run, cross)) = run_op(c) else {
        return Err(Error::Syntax(format!("expected a link near `{}`", c.rest())));
    };
    let mut label = String::new();
    let mut full = run.clone();
    let mut full_cross = cross;
    if matches!(run.as_str(), "--" | "==" | "-." | "<--" | "<==") && !cross {
        let rest: Vec<char> = c.s[c.i..].to_vec();
        let text: String = rest.iter().collect();
        let end = ["--", "==", ".-"].iter().filter_map(|p| text.find(p)).min();
        if let Some(e) = end {
            label = clean_label(&text[..e]);
            c.i += text[..e].chars().count();
            if let Some((r2, x2)) = run_op(c) {
                full = format!("{run}{r2}");
                full_cross = x2;
            }
        }
    }
    c.ws();
    if c.s.get(c.i) == Some(&'|') {
        if let Some(q) = c.s[c.i + 1..].iter().position(|&x| x == '|') {
            label = clean_label(&c.s[c.i + 1..c.i + 1 + q].iter().collect::<String>());
            c.i += q + 2;
        }
    }
    Ok(Op {
        label,
        dashed: full.contains('.'),
        thick: full.contains('='),
        head_to: full.ends_with('>') || full_cross,
        head_from: full.starts_with('<'),
    })
}

fn statement(st: &str, g: &mut Graph) -> Result<(), Error> {
    let chars: Vec<char> = st.chars().collect();
    let mut c = Cur { s: &chars, i: 0 };
    let mut prev = vec![parse_node(&mut c, g)?];
    loop {
        c.ws();
        if c.starts("&") {
            c.i += 1;
            let n = parse_node(&mut c, g)?;
            prev.push(n);
            continue;
        }
        if c.done() {
            return Ok(());
        }
        let op = parse_op(&mut c)?;
        let mut next = vec![parse_node(&mut c, g)?];
        loop {
            c.ws();
            if c.starts("&") {
                c.i += 1;
                next.push(parse_node(&mut c, g)?);
            } else {
                break;
            }
        }
        for &p in &prev {
            for &n in &next {
                g.edges.push(Edge {
                    from: p,
                    to: n,
                    label: op.label.clone(),
                    dashed: op.dashed,
                    thick: op.thick,
                    head_to: op.head_to,
                    head_from: op.head_from,
                });
            }
        }
        prev = next;
    }
}

pub fn render(header: &str, lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let dir = header
        .split_whitespace()
        .nth(1)
        .map(|d| d.trim_end_matches(';').to_ascii_uppercase())
        .unwrap_or_else(|| "TD".into());
    if !matches!(dir.as_str(), "TD" | "TB" | "LR" | "RL" | "BT") {
        return Err(Error::Syntax(format!("unknown direction {dir}")));
    }
    let mut g = Graph::default();
    for line in lines {
        for st in line.split(';') {
            let st = st.trim();
            if st.is_empty() {
                continue;
            }
            let kw = st.split_whitespace().next().unwrap_or("");
            match kw {
                "subgraph" => {
                    let title = st["subgraph".len()..].trim();
                    let title = match (title.find('['), title.rfind(']')) {
                        (Some(a), Some(b)) if b > a => clean_label(&title[a + 1..b]),
                        _ => clean_label(title),
                    };
                    g.groups.push((title, Vec::new()));
                    let idx = g.groups.len() - 1;
                    g.open.push(idx);
                }
                "end" => {
                    g.open.pop();
                }
                "direction" | "classDef" | "class" | "style" | "linkStyle" | "click" | "accTitle"
                | "accDescr" => {}
                _ => statement(st, &mut g)?,
            }
        }
    }
    if g.nodes.is_empty() {
        return Err(Error::Syntax("no nodes".into()));
    }
    Ok(layout(g, &dir, m))
}

fn node_size(n: &Node, m: &mut dyn Measure) -> (f32, f32) {
    let lines: Vec<&str> = n.label.split('\n').collect();
    let tw = lines.iter().map(|l| m.width(l, TEXT_SIZE, false)).fold(0.0, f32::max);
    let th = lines.len() as f32 * TEXT_SIZE * 1.3;
    match n.shape {
        Shape::Rect | Shape::Round => ((tw + 28.0).max(44.0), (th + 18.0).max(36.0)),
        Shape::Stadium => ((tw + 40.0).max(56.0), (th + 18.0).max(36.0)),
        Shape::Circle => {
            let d = (tw.max(th) + 22.0).max(44.0);
            (d, d)
        }
        Shape::Diamond => ((tw * 1.45 + 30.0).max(64.0), (th * 1.6 + 24.0).max(52.0)),
    }
}

fn layout(mut g: Graph, dir: &str, m: &mut dyn Measure) -> Diagram {
    let horizontal = matches!(dir, "LR" | "RL");
    for i in 0..g.nodes.len() {
        let (w, h) = node_size(&g.nodes[i], m);
        g.nodes[i].w = w;
        g.nodes[i].h = h;
    }
    let real = g.nodes.len();
    let n = real;

    // Break cycles: edges that close a cycle in DFS order are laid out reversed.
    let mut out: Vec<Vec<usize>> = vec![vec![]; n];
    for (k, e) in g.edges.iter().enumerate() {
        if e.from != e.to {
            out[e.from].push(k);
        }
    }
    let mut color = vec![0u8; n];
    let mut flipped = vec![false; g.edges.len()];
    for root in 0..n {
        if color[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        color[root] = 1;
        while let Some(&(v, k)) = stack.last() {
            if k < out[v].len() {
                let ei = out[v][k];
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                let to = g.edges[ei].to;
                match color[to] {
                    0 => {
                        color[to] = 1;
                        stack.push((to, 0));
                    }
                    1 => flipped[ei] = true,
                    _ => {}
                }
            } else {
                color[v] = 2;
                stack.pop();
            }
        }
    }
    let oriented: Vec<(usize, usize, usize)> = g
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.from != e.to)
        .map(|(k, e)| if flipped[k] { (e.to, e.from, k) } else { (e.from, e.to, k) })
        .collect();

    // Longest-path ranking.
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![vec![]; n];
    for &(u, v, _) in &oriented {
        succ[u].push(v);
        indeg[v] += 1;
    }
    let mut rank = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &v in &succ[u] {
            rank[v] = rank[v].max(rank[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }

    // Virtual nodes for edges that span several layers.
    let mut rank_of: Vec<usize> = rank.clone();
    let mut paths: Vec<(usize, Vec<usize>)> = Vec::new();
    for &(u, v, k) in &oriented {
        let mut path = vec![u];
        for r in rank[u] + 1..rank[v] {
            g.nodes.push(Node {
                id: String::new(),
                label: String::new(),
                shape: Shape::Rect,
                w: 6.0,
                h: 6.0,
                dummy: true,
            });
            rank_of.push(r);
            path.push(g.nodes.len() - 1);
        }
        path.push(v);
        paths.push((k, path));
    }
    let total = g.nodes.len();
    let nlayers = rank_of.iter().copied().max().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![vec![]; nlayers];
    for v in 0..total {
        layers[rank_of[v]].push(v);
    }
    let mut preds: Vec<Vec<usize>> = vec![vec![]; total];
    let mut succs: Vec<Vec<usize>> = vec![vec![]; total];
    for (_, p) in &paths {
        for w in p.windows(2) {
            succs[w[0]].push(w[1]);
            preds[w[1]].push(w[0]);
        }
    }
    let mut pos = vec![0.0f32; total];
    let reindex = |layers: &Vec<Vec<usize>>, pos: &mut Vec<f32>| {
        for l in layers {
            for (i, &v) in l.iter().enumerate() {
                pos[v] = i as f32;
            }
        }
    };
    reindex(&layers, &mut pos);
    for sweep in 0..8 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> =
            if down { (1..nlayers).collect() } else { (0..nlayers.saturating_sub(1)).rev().collect() };
        for l in order {
            let nb = if down { &preds } else { &succs };
            let mut keyed: Vec<(f32, usize)> = layers[l]
                .iter()
                .map(|&v| {
                    let ns = &nb[v];
                    let k = if ns.is_empty() {
                        pos[v]
                    } else {
                        ns.iter().map(|&x| pos[x]).sum::<f32>() / ns.len() as f32
                    };
                    (k, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            layers[l] = keyed.into_iter().map(|(_, v)| v).collect();
            reindex(&layers, &mut pos);
        }
    }

    // Cross-axis coordinates.
    let cross = |g: &Graph, v: usize| if horizontal { g.nodes[v].h } else { g.nodes[v].w };
    let flow = |g: &Graph, v: usize| if horizontal { g.nodes[v].w } else { g.nodes[v].h };
    let gap = 34.0;
    let mut c = vec![0.0f32; total];
    for l in &layers {
        let mut x = 0.0;
        for &v in l {
            c[v] = x + cross(&g, v) / 2.0;
            x += cross(&g, v) + gap;
        }
        let width = x - gap;
        for &v in l {
            c[v] -= width / 2.0;
        }
    }
    for pass in 0..6 {
        let down = pass % 2 == 0;
        let order: Vec<usize> =
            if down { (1..nlayers).collect() } else { (0..nlayers.saturating_sub(1)).rev().collect() };
        for l in order {
            let nb = if down { &preds } else { &succs };
            let layer = layers[l].clone();
            let want: Vec<f32> = layer
                .iter()
                .map(|&v| {
                    if nb[v].is_empty() {
                        c[v]
                    } else {
                        nb[v].iter().map(|&x| c[x]).sum::<f32>() / nb[v].len() as f32
                    }
                })
                .collect();
            let mut placed: Vec<f32> = Vec::with_capacity(layer.len());
            for (i, &v) in layer.iter().enumerate() {
                let mut x = want[i];
                if i > 0 {
                    let min = placed[i - 1] + (cross(&g, layer[i - 1]) + cross(&g, v)) / 2.0 + gap;
                    x = x.max(min);
                }
                placed.push(x);
            }
            let shift = if placed.is_empty() {
                0.0
            } else {
                placed.iter().zip(&want).map(|(p, w)| w - p).sum::<f32>() / placed.len() as f32
            };
            for (i, &v) in layer.iter().enumerate() {
                c[v] = placed[i] + shift;
            }
        }
    }

    // Flow-axis coordinates.
    let labelled = g.edges.iter().any(|e| !e.label.is_empty());
    let rank_gap = if labelled { 66.0 } else { 50.0 };
    let mut f = vec![0.0f32; total];
    let mut at = 0.0;
    for l in &layers {
        let t = l.iter().map(|&v| flow(&g, v)).fold(0.0, f32::max);
        for &v in l {
            f[v] = at + t / 2.0;
        }
        at += t + rank_gap;
    }
    let mut center: Vec<(f32, f32)> =
        (0..total).map(|v| if horizontal { (f[v], c[v]) } else { (c[v], f[v]) }).collect();
    if dir == "BT" {
        for p in &mut center {
            p.1 = -p.1;
        }
    }
    if dir == "RL" {
        for p in &mut center {
            p.0 = -p.0;
        }
    }

    let mut items: Vec<Item> = Vec::new();
    let mut lo = (f32::MAX, f32::MAX);
    let mut hi = (f32::MIN, f32::MIN);
    let mut grow = |x0: f32, y0: f32, x1: f32, y1: f32| {
        lo = (lo.0.min(x0), lo.1.min(y0));
        hi = (hi.0.max(x1), hi.1.max(y1));
    };

    for v in 0..real {
        let (cx, cy) = center[v];
        grow(
            cx - g.nodes[v].w / 2.0,
            cy - g.nodes[v].h / 2.0,
            cx + g.nodes[v].w / 2.0,
            cy + g.nodes[v].h / 2.0,
        );
    }

    // Subgraph frames.
    for (title, members) in &g.groups {
        if members.is_empty() {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &v in members {
            let (cx, cy) = center[v];
            x0 = x0.min(cx - g.nodes[v].w / 2.0);
            y0 = y0.min(cy - g.nodes[v].h / 2.0);
            x1 = x1.max(cx + g.nodes[v].w / 2.0);
            y1 = y1.max(cy + g.nodes[v].h / 2.0);
        }
        let (x0, y0, x1, y1) = (x0 - 16.0, y0 - 30.0, x1 + 16.0, y1 + 16.0);
        grow(x0, y0, x1, y1);
        items.push(Item::Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
            radius: 6.0,
            fill: Fill::Accent,
            stroke: true,
        });
        items.push(Item::Text {
            x: (x0 + x1) / 2.0,
            y: y0 + 14.0,
            text: title.clone(),
            bold: true,
            size: 12.0,
        });
    }

    // Edges.
    let clip = |g: &Graph, v: usize, c: (f32, f32), toward: (f32, f32)| -> (f32, f32) {
        let n = &g.nodes[v];
        let (dx, dy) = (toward.0 - c.0, toward.1 - c.1);
        if n.dummy || (dx.abs() < 1e-3 && dy.abs() < 1e-3) {
            return c;
        }
        let (hw, hh) = (n.w / 2.0, n.h / 2.0);
        let t = match n.shape {
            Shape::Rect | Shape::Round | Shape::Stadium => {
                (hw / dx.abs().max(1e-6)).min(hh / dy.abs().max(1e-6))
            }
            Shape::Diamond => 1.0 / (dx.abs() / hw + dy.abs() / hh),
            Shape::Circle => 1.0 / ((dx / hw).powi(2) + (dy / hh).powi(2)).sqrt(),
        };
        (c.0 + dx * t.min(1.0), c.1 + dy * t.min(1.0))
    };
    let mut labels: Vec<(f32, f32, String)> = Vec::new();
    let mut routed: Vec<usize> = Vec::new();
    for (k, path) in &paths {
        routed.push(*k);
        let e = &g.edges[*k];
        let mut idx = path.clone();
        if flipped[*k] {
            idx.reverse();
        }
        let mut pts: Vec<(f32, f32)> = idx.iter().map(|&v| center[v]).collect();
        let (a, b) = (idx[0], idx[idx.len() - 1]);
        let second = pts[1];
        let penult = pts[pts.len() - 2];
        pts[0] = clip(&g, a, center[a], second);
        let last = pts.len() - 1;
        pts[last] = clip(&g, b, center[b], penult);
        emit_edge(&mut items, &pts, e, &mut labels, &mut grow);
    }
    for (k, e) in g.edges.iter().enumerate() {
        if routed.contains(&k) {
            continue;
        }
        let (cx, cy) = center[e.from];
        let (hw, hh) = (g.nodes[e.from].w / 2.0, g.nodes[e.from].h / 2.0);
        let pts = vec![
            (cx + hw, cy - hh * 0.3),
            (cx + hw + 26.0, cy - hh * 0.3),
            (cx + hw + 26.0, cy + hh * 0.3),
            (cx + hw, cy + hh * 0.3),
        ];
        emit_edge(&mut items, &pts, e, &mut labels, &mut grow);
    }
    for (x, y, text) in labels {
        let w = m.width(&text, 12.0, false) + 10.0;
        grow(x - w / 2.0, y - 10.0, x + w / 2.0, y + 10.0);
        items.push(Item::Rect {
            x: x - w / 2.0,
            y: y - 10.0,
            w,
            h: 20.0,
            radius: 3.0,
            fill: Fill::Background,
            stroke: false,
        });
        items.push(Item::Text { x, y, text, bold: false, size: 12.0 });
    }

    // Nodes.
    for v in 0..real {
        let n = &g.nodes[v];
        let (cx, cy) = center[v];
        match n.shape {
            Shape::Rect => items.push(Item::Rect {
                x: cx - n.w / 2.0,
                y: cy - n.h / 2.0,
                w: n.w,
                h: n.h,
                radius: 3.0,
                fill: Fill::Node,
                stroke: true,
            }),
            Shape::Round => items.push(Item::Rect {
                x: cx - n.w / 2.0,
                y: cy - n.h / 2.0,
                w: n.w,
                h: n.h,
                radius: 10.0,
                fill: Fill::Node,
                stroke: true,
            }),
            Shape::Stadium => items.push(Item::Rect {
                x: cx - n.w / 2.0,
                y: cy - n.h / 2.0,
                w: n.w,
                h: n.h,
                radius: n.h / 2.0,
                fill: Fill::Node,
                stroke: true,
            }),
            Shape::Circle => {
                items.push(Item::Ellipse { cx, cy, rx: n.w / 2.0, ry: n.h / 2.0, fill: Fill::Node })
            }
            Shape::Diamond => items.push(Item::Diamond { cx, cy, w: n.w, h: n.h, fill: Fill::Node }),
        }
        let lines: Vec<&str> = n.label.split('\n').collect();
        let lh = TEXT_SIZE * 1.3;
        for (i, l) in lines.iter().enumerate() {
            let y = cy - (lines.len() as f32 - 1.0) * lh / 2.0 + i as f32 * lh;
            items.push(Item::Text { x: cx, y, text: (*l).to_string(), bold: false, size: TEXT_SIZE });
        }
    }

    let margin = 14.0;
    let (dx, dy) = (margin - lo.0, margin - lo.1);
    for it in &mut items {
        shift(it, dx, dy);
    }
    Diagram { width: hi.0 - lo.0 + 2.0 * margin, height: hi.1 - lo.1 + 2.0 * margin, items }
}

fn emit_edge(
    items: &mut Vec<Item>,
    pts: &[(f32, f32)],
    e: &Edge,
    labels: &mut Vec<(f32, f32, String)>,
    grow: &mut impl FnMut(f32, f32, f32, f32),
) {
    for p in pts {
        grow(p.0, p.1, p.0, p.1);
    }
    items.push(Item::Line { pts: pts.to_vec(), dashed: e.dashed, width: if e.thick { 3.0 } else { 1.5 } });
    let n = pts.len();
    let angle = |a: (f32, f32), b: (f32, f32)| (b.1 - a.1).atan2(b.0 - a.0);
    if e.head_to {
        items.push(Item::Arrow {
            x: pts[n - 1].0,
            y: pts[n - 1].1,
            angle: angle(pts[n - 2], pts[n - 1]),
            open: false,
        });
    }
    if e.head_from {
        items.push(Item::Arrow { x: pts[0].0, y: pts[0].1, angle: angle(pts[1], pts[0]), open: false });
    }
    if !e.label.is_empty() {
        let mut lens = Vec::new();
        let mut total = 0.0;
        for w in pts.windows(2) {
            let l = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
            lens.push(l);
            total += l;
        }
        let mut walk = total / 2.0;
        for (i, l) in lens.iter().enumerate() {
            if walk <= *l || i + 1 == lens.len() {
                let t = if *l > 0.0 { walk / l } else { 0.0 };
                let (a, b) = (pts[i], pts[i + 1]);
                labels.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, e.label.clone()));
                break;
            }
            walk -= l;
        }
    }
}

pub(crate) fn shift(it: &mut Item, dx: f32, dy: f32) {
    match it {
        Item::Rect { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Item::Diamond { cx, cy, .. } | Item::Ellipse { cx, cy, .. } => {
            *cx += dx;
            *cy += dy;
        }
        Item::Line { pts, .. } => {
            for p in pts {
                p.0 += dx;
                p.1 += dy;
            }
        }
        Item::Arrow { x, y, .. } | Item::Text { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
    }
}
