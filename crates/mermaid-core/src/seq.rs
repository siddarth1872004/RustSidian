use crate::{Diagram, Error, Fill, Item, Measure};

enum Ev {
    Msg { from: usize, to: usize, text: String, dashed: bool, head: bool, num: Option<usize> },
    Note { a: usize, b: usize, side: i8, text: String },
    Open(String),
    Divider(String),
    Close,
}

const OPS: &[(&str, bool, bool)] = &[
    ("-->>", true, true),
    ("->>", false, true),
    ("--x", true, true),
    ("-x", false, true),
    ("--)", true, true),
    ("-)", false, true),
    ("-->", true, false),
    ("->", false, false),
];

fn participant(names: &mut Vec<(String, String)>, id: &str) -> usize {
    match names.iter().position(|(i, _)| i == id) {
        Some(p) => p,
        None => {
            names.push((id.to_string(), id.to_string()));
            names.len() - 1
        }
    }
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut evs: Vec<Ev> = Vec::new();
    let mut auto = false;
    let mut counter = 0;
    let mut depth = 0usize;
    for line in lines {
        let kw = line.split_whitespace().next().unwrap_or("");
        let rest = line[kw.len()..].trim();
        match kw {
            "participant" | "actor" => {
                let (id, alias) = match rest.split_once(" as ") {
                    Some((i, a)) => (i.trim(), a.trim()),
                    None => (rest, rest),
                };
                let p = participant(&mut names, id);
                names[p].1 = alias.replace("<br/>", " ");
            }
            "autonumber" => auto = true,
            "title" | "activate" | "deactivate" | "box" | "link" | "links" | "create" | "destroy" => {}
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                depth += 1;
                evs.push(Ev::Open(format!("{kw} {rest}").trim().to_string()));
            }
            "else" | "and" | "option" => evs.push(Ev::Divider(rest.to_string())),
            "end" => {
                if depth > 0 {
                    depth -= 1;
                    evs.push(Ev::Close);
                }
            }
            "Note" | "note" => {
                let (head, text) =
                    rest.split_once(':').ok_or_else(|| Error::Syntax("note without text".into()))?;
                let head = head.trim();
                let (side, who) = if let Some(w) = head.strip_prefix("over ") {
                    (0, w)
                } else if let Some(w) = head.strip_prefix("left of ") {
                    (-1, w)
                } else if let Some(w) = head.strip_prefix("right of ") {
                    (1, w)
                } else {
                    return Err(Error::Syntax(format!("bad note: {line}")));
                };
                let mut it = who.split(',').map(str::trim);
                let a = participant(&mut names, it.next().unwrap_or(""));
                let b = it.next().map_or(a, |x| participant(&mut names, x));
                evs.push(Ev::Note {
                    a: a.min(b),
                    b: a.max(b),
                    side,
                    text: text.trim().replace("<br/>", "\n"),
                });
            }
            _ => {
                let Some((head, text)) = line.split_once(':') else {
                    return Err(Error::Syntax(format!("unexpected line: {line}")));
                };
                let found = OPS
                    .iter()
                    .filter_map(|&(op, dashed, head_)| head.find(op).map(|i| (i, op, dashed, head_)))
                    .min_by_key(|&(i, op, _, _)| (i, std::cmp::Reverse(op.len())));
                let Some((i, op, dashed, has_head)) = found else {
                    return Err(Error::Syntax(format!("unknown message: {line}")));
                };
                let from = head[..i].trim();
                let to = head[i + op.len()..].trim().trim_start_matches(['+', '-']);
                let (a, b) = (participant(&mut names, from), participant(&mut names, to));
                counter += 1;
                evs.push(Ev::Msg {
                    from: a,
                    to: b,
                    text: text.trim().replace("<br/>", " "),
                    dashed,
                    head: has_head,
                    num: auto.then_some(counter),
                });
            }
        }
    }
    if names.is_empty() {
        return Err(Error::Syntax("no participants".into()));
    }
    let n = names.len();
    let size = 13.0;
    let bw: Vec<f32> = names.iter().map(|(_, a)| (m.width(a, 14.0, true) + 26.0).max(72.0)).collect();
    let mut dist: Vec<f32> = (0..n.saturating_sub(1)).map(|i| (bw[i] + bw[i + 1]) / 2.0 + 24.0).collect();
    let mut needs: Vec<(usize, usize, f32)> = Vec::new();
    for e in &evs {
        match e {
            Ev::Msg { from, to, text, num, .. } if from != to => {
                let t = match num {
                    Some(k) => format!("{k}. {text}"),
                    None => text.clone(),
                };
                needs.push((*from.min(to), *from.max(to), m.width(&t, size, false) + 36.0));
            }
            Ev::Note { a, b, text, side: 0 } => {
                let w = text.lines().map(|l| m.width(l, size, false)).fold(0.0, f32::max) + 24.0;
                if a != b {
                    needs.push((*a, *b, w));
                }
            }
            _ => {}
        }
    }
    needs.sort_by_key(|&(a, b, _)| b - a);
    for (a, b, w) in needs {
        let cur: f32 = dist[a..b].iter().sum();
        if cur < w {
            dist[b - 1] += w - cur;
        }
    }
    let mut xs = vec![0.0f32; n];
    for i in 1..n {
        xs[i] = xs[i - 1] + dist[i - 1];
    }
    let mut items: Vec<Item> = Vec::new();
    let top = 36.0;
    let mut y = top + 26.0;
    let mut frames: Vec<(f32, String, usize)> = Vec::new();
    let mut frame_items: Vec<Item> = Vec::new();
    let mut body: Vec<Item> = Vec::new();
    let (x_min, x_max) = (xs[0] - bw[0] / 2.0 - 10.0, xs[n - 1] + bw[n - 1] / 2.0 + 10.0);
    let mut lo = x_min;
    let mut hi = x_max;
    for e in &evs {
        match e {
            Ev::Msg { from, to, text, dashed, head, num } => {
                let t = match num {
                    Some(k) => format!("{k}. {text}"),
                    None => text.clone(),
                };
                if from == to {
                    let x = xs[*from];
                    let pts = vec![(x, y), (x + 34.0, y), (x + 34.0, y + 18.0), (x, y + 18.0)];
                    body.push(Item::Line { pts, dashed: *dashed, width: 1.5 });
                    if *head {
                        body.push(Item::Arrow { x, y: y + 18.0, angle: std::f32::consts::PI, open: false });
                    }
                    let w = m.width(&t, size, false);
                    body.push(Item::Text { x: x + 42.0 + w / 2.0, y: y + 9.0, text: t, bold: false, size });
                    hi = hi.max(x + 50.0 + w);
                    y += 40.0;
                } else {
                    let (x0, x1) = (xs[*from], xs[*to]);
                    body.push(Item::Line { pts: vec![(x0, y), (x1, y)], dashed: *dashed, width: 1.5 });
                    if *head {
                        body.push(Item::Arrow {
                            x: x1,
                            y,
                            angle: if x1 > x0 { 0.0 } else { std::f32::consts::PI },
                            open: false,
                        });
                    }
                    body.push(Item::Text { x: (x0 + x1) / 2.0, y: y - 10.0, text: t, bold: false, size });
                    y += 38.0;
                }
            }
            Ev::Note { a, b, side, text } => {
                let lines: Vec<&str> = text.lines().collect();
                let w = lines.iter().map(|l| m.width(l, size, false)).fold(0.0, f32::max) + 20.0;
                let h = lines.len() as f32 * 17.0 + 12.0;
                let (x0, ww) = match side {
                    0 => {
                        let (l, r) = (xs[*a], xs[*b]);
                        let ww = w.max(r - l + 30.0);
                        ((l + r) / 2.0 - ww / 2.0, ww)
                    }
                    -1 => (xs[*a] - 12.0 - w, w),
                    _ => (xs[*a] + 12.0, w),
                };
                lo = lo.min(x0 - 4.0);
                hi = hi.max(x0 + ww + 4.0);
                body.push(Item::Rect { x: x0, y, w: ww, h, radius: 2.0, fill: Fill::Accent, stroke: true });
                for (i, l) in lines.iter().enumerate() {
                    body.push(Item::Text {
                        x: x0 + ww / 2.0,
                        y: y + 6.0 + 8.5 + i as f32 * 17.0,
                        text: (*l).to_string(),
                        bold: false,
                        size,
                    });
                }
                y += h + 16.0;
            }
            Ev::Open(title) => {
                frames.push((y - 8.0, title.clone(), frames.len()));
                y += 22.0;
            }
            Ev::Divider(t) => {
                let inset = frames.len() as f32 * 6.0;
                frame_items.push(Item::Line {
                    pts: vec![(x_min + inset, y - 6.0), (x_max - inset, y - 6.0)],
                    dashed: true,
                    width: 1.0,
                });
                if !t.is_empty() {
                    let w = m.width(&format!("[{t}]"), 12.0, false);
                    frame_items.push(Item::Text {
                        x: x_min + inset + 14.0 + w / 2.0,
                        y: y + 6.0,
                        text: format!("[{t}]"),
                        bold: false,
                        size: 12.0,
                    });
                }
                y += 22.0;
            }
            Ev::Close => {
                if let Some((y0, title, d)) = frames.pop() {
                    let inset = d as f32 * 6.0;
                    frame_items.insert(
                        0,
                        Item::Rect {
                            x: x_min + inset,
                            y: y0,
                            w: x_max - x_min - 2.0 * inset,
                            h: y - y0 + 4.0,
                            radius: 3.0,
                            fill: Fill::None,
                            stroke: true,
                        },
                    );
                    let w = m.width(&title, 12.0, true);
                    frame_items.push(Item::Text {
                        x: x_min + inset + 8.0 + w / 2.0,
                        y: y0 + 11.0,
                        text: title,
                        bold: true,
                        size: 12.0,
                    });
                    y += 10.0;
                }
            }
        }
    }
    while let Some((y0, title, d)) = frames.pop() {
        let inset = d as f32 * 6.0;
        frame_items.push(Item::Rect {
            x: x_min + inset,
            y: y0,
            w: x_max - x_min - 2.0 * inset,
            h: y - y0,
            radius: 3.0,
            fill: Fill::None,
            stroke: true,
        });
        frame_items.push(Item::Text {
            x: x_min + inset + 30.0,
            y: y0 + 11.0,
            text: title,
            bold: true,
            size: 12.0,
        });
    }
    let bottom = y + 10.0;
    items.extend(frame_items);
    for i in 0..n {
        items.push(Item::Line { pts: vec![(xs[i], top), (xs[i], bottom)], dashed: true, width: 1.0 });
    }
    items.extend(body);
    for i in 0..n {
        for by in [0.0, bottom] {
            items.push(Item::Rect {
                x: xs[i] - bw[i] / 2.0,
                y: by,
                w: bw[i],
                h: 32.0,
                radius: 4.0,
                fill: Fill::Node,
                stroke: true,
            });
            items.push(Item::Text {
                x: xs[i],
                y: by + 16.0,
                text: names[i].1.clone(),
                bold: true,
                size: 14.0,
            });
        }
    }
    let margin = 12.0;
    let dx = margin - lo;
    for it in &mut items {
        crate::flow::shift(it, dx, margin);
    }
    Ok(Diagram { width: hi - lo + 2.0 * margin, height: bottom + 32.0 + 2.0 * margin, items })
}
