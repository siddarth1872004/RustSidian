use crate::syms::{self, Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Roman,
    Italic,
    Bold,
    Mono,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Center,
    Aligned,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Node {
    Atom { text: String, kind: Kind, face: Face },
    Group(Vec<Node>),
    Script { base: Box<Node>, sup: Option<Vec<Node>>, sub: Option<Vec<Node>> },
    Frac { num: Vec<Node>, den: Vec<Node>, rule: bool, forced: Option<bool> },
    Sqrt { body: Vec<Node>, index: Option<Vec<Node>> },
    Delim { open: String, close: String, body: Vec<Node> },
    Text(String),
    Space(f32),
    Matrix { rows: Vec<Vec<Vec<Node>>>, open: String, close: String, align: Align },
    Accent { kind: &'static str, body: Vec<Node> },
    Line { over: bool, body: Vec<Node> },
    Unknown(String),
}

#[derive(PartialEq, Debug)]
enum Term {
    End,
    Brace,
    Amp,
    Cr,
    EndEnv(String),
    Right(String),
}

pub fn parse(src: &str) -> Vec<Node> {
    let mut p = Parser { c: src.chars().collect(), i: 0, depth: 0 };
    let mut out = Vec::new();
    loop {
        let (mut nodes, term) = p.seq(None);
        out.append(&mut nodes);
        if term == Term::End || p.i >= p.c.len() {
            return out;
        }
    }
}

struct Parser {
    c: Vec<char>,
    i: usize,
    depth: usize,
}

fn atom(text: impl Into<String>, kind: Kind, face: Face) -> Node {
    Node::Atom { text: text.into(), kind, face }
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn name(&mut self) -> String {
        let s = self.i;
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.i += 1;
            }
        } else if self.peek().is_some() {
            self.i += 1;
        }
        self.c[s..self.i].iter().collect()
    }

    fn raw_group(&mut self) -> String {
        self.skip_ws();
        if self.peek() != Some('{') {
            return self.name();
        }
        self.i += 1;
        let s = self.i;
        let mut d = 1;
        while let Some(c) = self.peek() {
            match c {
                '{' => d += 1,
                '}' => {
                    d -= 1;
                    if d == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        let r = self.c[s..self.i.min(self.c.len())].iter().collect();
        self.i += 1;
        r
    }

    fn arg(&mut self, face: Option<Face>) -> Vec<Node> {
        self.skip_ws();
        match self.peek() {
            Some('{') => {
                self.i += 1;
                self.seq(face).0
            }
            Some('\\') => {
                self.i += 1;
                let n = self.name();
                vec![self.command(&n, face)]
            }
            Some(c) => {
                self.i += 1;
                vec![self.ch(c, face)]
            }
            None => vec![],
        }
    }

    fn ch(&self, c: char, face: Option<Face>) -> Node {
        let s = c.to_string();
        match c {
            'a'..='z' | 'A'..='Z' => atom(s, Kind::Ord, face.unwrap_or(Face::Italic)),
            '0'..='9' | '.' => atom(s, Kind::Ord, face.unwrap_or(Face::Roman)),
            '+' | '*' => atom(if c == '*' { "∗".into() } else { s }, Kind::Bin, Face::Roman),
            '-' => atom("−", Kind::Bin, Face::Roman),
            '=' | '<' | '>' => atom(s, Kind::Rel, Face::Roman),
            ':' => atom(s, Kind::Rel, Face::Roman),
            '(' | '[' => atom(s, Kind::Open, Face::Roman),
            ')' | ']' => atom(s, Kind::Close, Face::Roman),
            ',' | ';' => atom(s, Kind::Punct, Face::Roman),
            '|' => atom(s, Kind::Ord, Face::Roman),
            '\'' => atom("′", Kind::Ord, Face::Roman),
            c if c.is_alphabetic() => atom(s, Kind::Ord, face.unwrap_or(Face::Italic)),
            _ => atom(s, Kind::Ord, face.unwrap_or(Face::Roman)),
        }
    }

    fn seq(&mut self, face: Option<Face>) -> (Vec<Node>, Term) {
        self.depth += 1;
        let r = if self.depth > 60 { (Vec::new(), Term::End) } else { self.seq_inner(face) };
        self.depth -= 1;
        r
    }

    fn seq_inner(&mut self, face: Option<Face>) -> (Vec<Node>, Term) {
        let mut out: Vec<Node> = Vec::new();
        loop {
            let Some(c) = self.peek() else { return (out, Term::End) };
            self.i += 1;
            match c {
                c if c.is_whitespace() => {}
                '{' => {
                    let (g, t) = self.seq(face);
                    out.push(Node::Group(g));
                    if t != Term::Brace {
                        return (out, t);
                    }
                }
                '}' => return (out, Term::Brace),
                '&' => return (out, Term::Amp),
                '^' | '_' => {
                    let arg = self.arg(face);
                    let base = out.pop().unwrap_or(Node::Group(vec![]));
                    let (mut b, mut sup, mut sub) = match base {
                        Node::Script { base, sup, sub } => (*base, sup, sub),
                        other => (other, None, None),
                    };
                    let slot = if c == '^' { &mut sup } else { &mut sub };
                    if slot.is_some() {
                        b = Node::Script { base: Box::new(b), sup: sup.take(), sub: sub.take() };
                        let (s1, s2) = if c == '^' { (Some(arg), None) } else { (None, Some(arg)) };
                        out.push(Node::Script { base: Box::new(b), sup: s1, sub: s2 });
                    } else {
                        *slot = Some(arg);
                        out.push(Node::Script { base: Box::new(b), sup, sub });
                    }
                }
                '~' => out.push(Node::Space(0.33)),
                '%' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '\\' => {
                    let n = self.name();
                    match n.as_str() {
                        "\\" | "cr" => return (out, Term::Cr),
                        "right" => {
                            let d = self.delim();
                            return (out, Term::Right(d));
                        }
                        "end" => {
                            let e = self.raw_group();
                            return (out, Term::EndEnv(e));
                        }
                        _ => {
                            let node = self.command(&n, face);
                            out.push(node);
                        }
                    }
                }
                c => out.push(self.ch(c, face)),
            }
        }
    }

    fn delim(&mut self) -> String {
        self.skip_ws();
        match self.peek() {
            Some('\\') => {
                self.i += 1;
                let n = self.name();
                match n.as_str() {
                    "{" | "lbrace" => "{".into(),
                    "}" | "rbrace" => "}".into(),
                    "|" | "Vert" | "lVert" | "rVert" => "‖".into(),
                    "vert" | "lvert" | "rvert" => "|".into(),
                    "langle" => "⟨".into(),
                    "rangle" => "⟩".into(),
                    "lfloor" => "⌊".into(),
                    "rfloor" => "⌋".into(),
                    "lceil" => "⌈".into(),
                    "rceil" => "⌉".into(),
                    _ => String::new(),
                }
            }
            Some('.') => {
                self.i += 1;
                String::new()
            }
            Some(c) => {
                self.i += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }

    fn command(&mut self, n: &str, face: Option<Face>) -> Node {
        if let Some((s, kind)) = syms::lookup(n) {
            let f = if syms::is_greek_lower(s) { face.unwrap_or(Face::Italic) } else { Face::Roman };
            return atom(s, kind, f);
        }
        if syms::FUNCS.contains(&n) {
            return Node::Group(vec![atom(n, Kind::Op, Face::Roman), Node::Space(0.17)]);
        }
        match n {
            "," => Node::Space(0.17),
            ":" | ">" => Node::Space(0.22),
            ";" => Node::Space(0.28),
            "!" => Node::Space(-0.17),
            " " => Node::Space(0.28),
            "quad" => Node::Space(1.0),
            "qquad" => Node::Space(2.0),
            "{" => atom("{", Kind::Open, Face::Roman),
            "}" => atom("}", Kind::Close, Face::Roman),
            "|" => atom("‖", Kind::Ord, Face::Roman),
            "%" | "$" | "&" | "#" | "_" => atom(n, Kind::Ord, Face::Roman),
            "displaystyle" | "textstyle" | "scriptstyle" | "limits" | "nolimits" | "left" => {
                if n == "left" {
                    return self.left(face);
                }
                Node::Space(0.0)
            }
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr"
            | "bigm" => {
                let d = self.delim();
                atom(d, Kind::Ord, Face::Roman)
            }
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.arg(face);
                let den = self.arg(face);
                let forced = match n {
                    "dfrac" => Some(true),
                    "tfrac" => Some(false),
                    _ => None,
                };
                Node::Frac { num, den, rule: true, forced }
            }
            "binom" | "dbinom" | "tbinom" => {
                let num = self.arg(face);
                let den = self.arg(face);
                Node::Delim {
                    open: "(".into(),
                    close: ")".into(),
                    body: vec![Node::Frac { num, den, rule: false, forced: None }],
                }
            }
            "sqrt" => {
                self.skip_ws();
                let index = if self.peek() == Some('[') {
                    self.i += 1;
                    let s = self.i;
                    while self.peek().is_some_and(|c| c != ']') {
                        self.i += 1;
                    }
                    let sub: String = self.c[s..self.i].iter().collect();
                    self.i += 1;
                    Some(parse(&sub))
                } else {
                    None
                };
                Node::Sqrt { body: self.arg(face), index }
            }
            "text" | "textrm" | "mbox" | "textnormal" | "textit" | "textbf" | "operatorname" | "mathrm"
            | "mathit" | "mathbf" | "boldsymbol" | "bm" | "mathbb" | "mathcal" | "mathscr" | "mathfrak"
            | "mathsf" | "mathtt" => {
                let f = match n {
                    "mathbf" | "boldsymbol" | "bm" | "textbf" => Face::Bold,
                    "mathit" | "textit" => Face::Italic,
                    "mathtt" => Face::Mono,
                    _ => Face::Roman,
                };
                if matches!(
                    n,
                    "text" | "textrm" | "mbox" | "textnormal" | "textit" | "textbf" | "operatorname"
                ) {
                    let t = self.raw_group();
                    return Node::Atom {
                        text: t,
                        kind: if n == "operatorname" { Kind::Op } else { Kind::Ord },
                        face: if n == "text" || n == "mbox" { Face::Roman } else { f },
                    }
                    .into_text(n != "operatorname");
                }
                let mut body = self.arg(Some(f));
                let map: Option<fn(char) -> char> = match n {
                    "mathbb" => Some(syms::blackboard),
                    "mathcal" | "mathscr" => Some(syms::calligraphic),
                    "mathfrak" => Some(syms::fraktur),
                    _ => None,
                };
                if let Some(m) = map {
                    map_text(&mut body, m);
                }
                Node::Group(body)
            }
            "hat" | "widehat" => Node::Accent { kind: "^", body: self.arg(face) },
            "tilde" | "widetilde" => Node::Accent { kind: "~", body: self.arg(face) },
            "bar" => Node::Accent { kind: "¯", body: self.arg(face) },
            "vec" => Node::Accent { kind: "→", body: self.arg(face) },
            "dot" => Node::Accent { kind: "˙", body: self.arg(face) },
            "ddot" => Node::Accent { kind: "¨", body: self.arg(face) },
            "overline" => Node::Line { over: true, body: self.arg(face) },
            "underline" => Node::Line { over: false, body: self.arg(face) },
            "begin" => self.environment(),
            "mathbin" | "mathrel" | "mathop" | "mathord" => Node::Group(self.arg(face)),
            _ => Node::Unknown(format!("\\{n}")),
        }
    }

    fn left(&mut self, face: Option<Face>) -> Node {
        let open = self.delim();
        let (body, t) = self.seq(face);
        let close = match t {
            Term::Right(d) => d,
            _ => String::new(),
        };
        Node::Delim { open, close, body }
    }

    fn environment(&mut self) -> Node {
        let env = self.raw_group();
        let (open, close, align) = match env.trim_end_matches('*') {
            "pmatrix" => ("(", ")", Align::Center),
            "bmatrix" => ("[", "]", Align::Center),
            "Bmatrix" => ("{", "}", Align::Center),
            "vmatrix" => ("|", "|", Align::Center),
            "Vmatrix" => ("‖", "‖", Align::Center),
            "matrix" | "smallmatrix" | "array" | "gathered" | "gather" => ("", "", Align::Center),
            "cases" => ("{", "", Align::Aligned),
            "aligned" | "align" | "split" | "alignedat" | "eqnarray" => ("", "", Align::Aligned),
            _ => {
                let end = format!("\\end{{{env}}}");
                let rest: String = self.c[self.i..].iter().collect();
                let src = match rest.find(&end) {
                    Some(p) => {
                        self.i += rest[..p].chars().count() + end.chars().count();
                        &rest[..p]
                    }
                    None => {
                        self.i = self.c.len();
                        &rest[..]
                    }
                };
                return Node::Unknown(format!("\\begin{{{env}}}{}\\end{{{env}}}", src.trim()));
            }
        };
        if env == "array" {
            self.raw_group();
        }
        let mut rows: Vec<Vec<Vec<Node>>> = vec![vec![]];
        loop {
            let (cell, t) = self.seq(None);
            rows.last_mut().unwrap().push(cell);
            match t {
                Term::Amp => {}
                Term::Cr => rows.push(vec![]),
                _ => break,
            }
        }
        if rows.last().is_some_and(|r| r.len() == 1 && r[0].is_empty()) && rows.len() > 1 {
            rows.pop();
        }
        Node::Matrix { rows, open: open.into(), close: close.into(), align }
    }
}

impl Node {
    fn into_text(self, plain: bool) -> Node {
        match self {
            Node::Atom { text, kind, .. } => {
                if plain {
                    Node::Text(text)
                } else {
                    Node::Atom { text, kind, face: Face::Roman }
                }
            }
            n => n,
        }
    }
}

fn map_text(nodes: &mut [Node], f: fn(char) -> char) {
    for n in nodes {
        match n {
            Node::Atom { text, .. } => *text = text.chars().map(f).collect(),
            Node::Group(g) => map_text(g, f),
            _ => {}
        }
    }
}
