//! Persisted index: `.walnut/cache/index.bin`.
//!
//! Layout (little endian): magic, version, payload length, payload checksum, then the payload:
//! a file table (path, stamp, metadata offset+length) followed by the metadata blobs. The file is
//! memory-mapped and stays mapped: only the table is read at open, and a note's metadata blob is
//! decoded when something asks for it. A bad header, checksum or table makes `load` return
//! `None` and the vault is re-indexed from disk.

use crate::fsio::{atomic_write, fold64, Stamp};
use crate::meta::*;
use crate::mmap::Mapped;
use md_engine::frontmatter::Value;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"WNIX";
const VERSION: u32 = 4;
const HEADER: usize = 24;

pub struct CachedFile {
    pub path: String,
    pub stamp: Stamp,
    pub meta: Option<Blob>,
}

/// Bit flags selecting which parts of a note's metadata to decode.
pub mod parts {
    pub const LINKS: u8 = 1;
    pub const TAGS: u8 = 2;
    pub const HEADINGS: u8 = 4;
    pub const BLOCKS: u8 = 8;
    pub const SECTIONS: u8 = 16;
    pub const LIST_ITEMS: u8 = 32;
    /// Properties, aliases and cssclasses.
    pub const FRONTMATTER: u8 = 64;
    pub const ALL: u8 = 0xff;
}

/// One note's encoded metadata inside the mapped index.
#[derive(Clone)]
pub struct Blob {
    map: Arc<Mapped>,
    off: usize,
    len: usize,
}

impl Blob {
    pub fn bytes(&self) -> &[u8] {
        &self.map.bytes()[self.off..self.off + self.len]
    }

    pub fn decode(&self) -> Option<NoteMeta> {
        self.decode_parts(parts::ALL)
    }

    /// Decode only the selected [`parts`]; the others are left empty.
    pub fn decode_parts(&self, parts: u8) -> Option<NoteMeta> {
        let b = self.bytes();
        let mut r = R { b, i: 0, depth: 0 };
        let m = r.meta(parts)?;
        (r.i == b.len()).then_some(m)
    }

    /// Visit the link targets without decoding or allocating anything else.
    pub fn link_targets(&self, f: &mut dyn FnMut(&str)) {
        let mut r = R { b: self.bytes(), i: 0, depth: 0 };
        let mut go = || -> Option<()> {
            for _ in 0..r.count(30)? {
                f(r.str()?);
                r.str()?;
                r.str()?;
                r.take(18)?;
            }
            Some(())
        };
        go();
    }
}

/// A note's metadata: decoded in memory (new or changed notes), or still encoded in the index.
#[derive(Clone)]
pub enum Meta {
    Owned(Arc<NoteMeta>),
    Cached(Blob),
}

impl Meta {
    /// The decoded metadata. For `Cached` this decodes on every call; hold on to the result.
    /// The index is checksummed at load, so a blob that fails to decode means a bug, and the
    /// note then reads as having no metadata until it is next re-indexed.
    pub fn get(&self) -> Arc<NoteMeta> {
        self.get_parts(parts::ALL)
    }

    /// Like `get`, but a `Cached` note only decodes the selected [`parts`]. Scalar fields
    /// (word count, front matter end and error) are always present.
    pub fn get_parts(&self, parts: u8) -> Arc<NoteMeta> {
        match self {
            Meta::Owned(m) => m.clone(),
            Meta::Cached(b) => Arc::new(b.decode_parts(parts).unwrap_or_default()),
        }
    }

    pub fn link_targets(&self, f: &mut dyn FnMut(&str)) {
        match self {
            Meta::Owned(m) => m.links.iter().for_each(|l| f(&l.target)),
            Meta::Cached(b) => b.link_targets(f),
        }
    }
}

impl std::fmt::Debug for Meta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Meta::Owned(m) => m.fmt(f),
            Meta::Cached(b) => write!(f, "Cached({} bytes)", b.len),
        }
    }
}

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn value(&mut self, v: &Value) {
        match v {
            Value::Null => self.u8(0),
            Value::Bool(b) => {
                self.u8(1);
                self.u8(*b as u8);
            }
            Value::Number(n) => {
                self.u8(2);
                self.u64(n.to_bits());
            }
            Value::String(s) => {
                self.u8(3);
                self.str(s);
            }
            Value::List(items) => {
                self.u8(4);
                self.u32(items.len() as u32);
                for i in items {
                    self.value(i);
                }
            }
            Value::Map(items) => {
                self.u8(5);
                self.u32(items.len() as u32);
                for (k, v) in items {
                    self.str(k);
                    self.value(v);
                }
            }
        }
    }
    fn meta(&mut self, m: &NoteMeta) {
        self.u32(m.links.len() as u32);
        for l in &m.links {
            self.str(&l.target);
            self.str(&l.subpath);
            self.str(&l.display);
            for v in [l.start, l.end, l.target_start, l.target_end] {
                self.u32(v);
            }
            self.u8(l.embed as u8);
            self.u8(match l.kind {
                LinkKind::Wiki => 0,
                LinkKind::Markdown => 1,
                LinkKind::FrontMatter => 2,
            });
        }
        self.u32(m.tags.len() as u32);
        for t in &m.tags {
            self.str(&t.tag);
            self.u32(t.start);
        }
        self.u32(m.headings.len() as u32);
        for h in &m.headings {
            self.u8(h.level);
            self.str(&h.text);
            self.u32(h.start);
            self.u32(h.end);
        }
        self.u32(m.blocks.len() as u32);
        for b in &m.blocks {
            self.str(&b.id);
            self.u32(b.start);
            self.u32(b.end);
        }
        self.u32(m.sections.len() as u32);
        for s in &m.sections {
            self.u8(s.kind as u8);
            self.u32(s.start);
            self.u32(s.end);
        }
        self.u32(m.list_items.len() as u32);
        for i in &m.list_items {
            self.u32(i.start);
            self.u32(i.end);
            self.u8(i.task.unwrap_or(0));
            self.u8(i.task.is_some() as u8);
            self.u32(i.parent as u32);
        }
        match &m.frontmatter {
            Some(p) => {
                self.u8(1);
                self.u32(p.len() as u32);
                for (k, v) in p {
                    self.str(k);
                    self.value(v);
                }
            }
            None => self.u8(0),
        }
        self.str(m.frontmatter_error.as_deref().unwrap_or(""));
        self.u32(m.frontmatter_end);
        self.u32(m.aliases.len() as u32);
        for a in &m.aliases {
            self.str(a);
        }
        self.u32(m.cssclasses.len() as u32);
        for c in &m.cssclasses {
            self.str(c);
        }
        self.u32(m.words);
    }
}

struct R<'a> {
    b: &'a [u8],
    i: usize,
    depth: u32,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i.checked_add(n)?)?;
        self.i += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<&'a str> {
        let n = self.u32()? as usize;
        std::str::from_utf8(self.take(n)?).ok()
    }
    fn boxed(&mut self) -> Option<Box<str>> {
        self.str().map(Into::into)
    }
    /// Element count, sanity-checked against the remaining bytes so that a corrupt count cannot
    /// trigger a huge allocation.
    fn count(&mut self, min_elem: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n.checked_mul(min_elem)? <= self.b.len() - self.i).then_some(n)
    }
    fn value(&mut self) -> Option<Value> {
        self.depth += 1;
        if self.depth > 80 {
            return None;
        }
        let v = match self.u8()? {
            0 => Value::Null,
            1 => Value::Bool(self.u8()? != 0),
            2 => Value::Number(f64::from_bits(self.u64()?)),
            3 => Value::String(self.str()?.to_string()),
            4 => {
                let n = self.count(1)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.value()?);
                }
                Value::List(items)
            }
            5 => {
                let n = self.count(5)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push((self.str()?.to_string(), self.value()?));
                }
                Value::Map(items)
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(v)
    }
    fn skip_value(&mut self) -> Option<()> {
        self.depth += 1;
        if self.depth > 80 {
            return None;
        }
        match self.u8()? {
            0 => {}
            1 => {
                self.u8()?;
            }
            2 => {
                self.u64()?;
            }
            3 => {
                self.str()?;
            }
            4 => {
                for _ in 0..self.count(1)? {
                    self.skip_value()?;
                }
            }
            5 => {
                for _ in 0..self.count(5)? {
                    self.str()?;
                    self.skip_value()?;
                }
            }
            _ => return None,
        }
        self.depth -= 1;
        Some(())
    }
    /// Decode the parts of a note's metadata selected by `parts`; the rest is skipped without
    /// allocating and left empty.
    fn meta(&mut self, parts: u8) -> Option<NoteMeta> {
        use parts::*;
        let mut m = NoteMeta::default();
        let want = |p: u8| parts & p != 0;
        for _ in 0..self.count(30)? {
            if !want(LINKS) {
                self.str()?;
                self.str()?;
                self.str()?;
                self.take(18)?;
                continue;
            }
            m.links.push(Link {
                target: self.boxed()?,
                subpath: self.boxed()?,
                display: self.boxed()?,
                start: self.u32()?,
                end: self.u32()?,
                target_start: self.u32()?,
                target_end: self.u32()?,
                embed: self.u8()? != 0,
                kind: match self.u8()? {
                    0 => LinkKind::Wiki,
                    1 => LinkKind::Markdown,
                    2 => LinkKind::FrontMatter,
                    _ => return None,
                },
            });
        }
        for _ in 0..self.count(8)? {
            let (tag, start) = (self.str()?, self.u32()?);
            if want(TAGS) {
                m.tags.push(TagRef { tag: tag.into(), start });
            }
        }
        for _ in 0..self.count(13)? {
            let (level, text, start, end) = (self.u8()?, self.str()?, self.u32()?, self.u32()?);
            if want(HEADINGS) {
                m.headings.push(Heading { level, text: text.into(), start, end });
            }
        }
        for _ in 0..self.count(12)? {
            let (id, start, end) = (self.str()?, self.u32()?, self.u32()?);
            if want(BLOCKS) {
                m.blocks.push(BlockRef { id: id.into(), start, end });
            }
        }
        let n = self.count(9)?;
        if want(SECTIONS) {
            m.sections.reserve_exact(n);
            for _ in 0..n {
                let kind = section_kind(self.u8()?)?;
                m.sections.push(Section { kind, start: self.u32()?, end: self.u32()? });
            }
        } else {
            self.take(n * 9)?;
        }
        let n = self.count(14)?;
        if want(LIST_ITEMS) {
            m.list_items.reserve_exact(n);
            for _ in 0..n {
                let (start, end, ch, has) = (self.u32()?, self.u32()?, self.u8()?, self.u8()?);
                m.list_items.push(ListItem {
                    start,
                    end,
                    task: (has != 0).then_some(ch),
                    parent: self.u32()? as i32,
                });
            }
        } else {
            self.take(n * 14)?;
        }
        if self.u8()? != 0 {
            let n = self.count(5)?;
            if want(FRONTMATTER) {
                let mut p = Vec::with_capacity(n);
                for _ in 0..n {
                    p.push((self.str()?.to_string(), self.value()?));
                }
                m.frontmatter = Some(p);
            } else {
                for _ in 0..n {
                    self.str()?;
                    self.skip_value()?;
                }
            }
        }
        let err = self.str()?;
        m.frontmatter_error = (!err.is_empty()).then(|| err.to_string());
        m.frontmatter_end = self.u32()?;
        for _ in 0..self.count(4)? {
            let a = self.str()?;
            if want(FRONTMATTER) {
                m.aliases.push(a.into());
            }
        }
        for _ in 0..self.count(4)? {
            let c = self.str()?;
            if want(FRONTMATTER) {
                m.cssclasses.push(c.into());
            }
        }
        m.words = self.u32()?;
        Some(m)
    }
}

fn section_kind(v: u8) -> Option<SectionKind> {
    use SectionKind::*;
    const ALL: [SectionKind; 14] = [
        FrontMatter,
        Heading,
        Paragraph,
        Code,
        Math,
        Quote,
        Callout,
        List,
        Table,
        Rule,
        Html,
        Footnote,
        Comment,
        Definition,
    ];
    ALL.get(v as usize).copied()
}

pub fn encode<'a>(files: impl Iterator<Item = (&'a str, Stamp, Option<&'a Meta>)>) -> Vec<u8> {
    let mut table = W::default();
    let mut blobs = W::default();
    let mut n = 0u32;
    for (path, stamp, meta) in files {
        n += 1;
        table.str(path);
        table.u64(stamp.mtime as u64);
        table.u64(stamp.size);
        match meta {
            Some(m) => {
                let start = blobs.0.len();
                match m {
                    Meta::Owned(m) => blobs.meta(m),
                    Meta::Cached(b) => blobs.0.extend_from_slice(b.bytes()),
                }
                table.u64(start as u64);
                table.u32((blobs.0.len() - start) as u32);
            }
            None => {
                table.u64(u64::MAX);
                table.u32(0);
            }
        }
    }
    let mut payload = W::default();
    payload.u32(n);
    payload.u64(table.0.len() as u64);
    payload.0.extend_from_slice(&table.0);
    payload.0.extend_from_slice(&blobs.0);
    let mut out = Vec::with_capacity(HEADER + payload.0.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(payload.0.len() as u64).to_le_bytes());
    out.extend_from_slice(&fold64(&payload.0).to_le_bytes());
    out.extend_from_slice(&payload.0);
    out
}

pub fn decode(map: Arc<Mapped>) -> Option<Vec<CachedFile>> {
    let bytes = map.bytes();
    if bytes.len() < HEADER || &bytes[..4] != MAGIC || bytes[4..8] != VERSION.to_le_bytes() {
        return None;
    }
    let len = u64::from_le_bytes(bytes[8..16].try_into().ok()?) as usize;
    let sum = u64::from_le_bytes(bytes[16..24].try_into().ok()?);
    let payload = bytes.get(HEADER..HEADER.checked_add(len)?)?;
    if bytes.len() != HEADER + len || fold64(payload) != sum {
        return None;
    }
    let mut r = R { b: payload, i: 0, depth: 0 };
    let n = r.count(28)?;
    let table_len = r.u64()? as usize;
    let blobs_at = r.i.checked_add(table_len)?;
    let blobs_len = payload.len().checked_sub(blobs_at)?;
    let mut files = Vec::with_capacity(n);
    for _ in 0..n {
        let path = r.str()?.to_string();
        let stamp = Stamp { mtime: r.u64()? as i64, size: r.u64()? };
        let (off, blen) = (r.u64()?, r.u32()? as usize);
        let meta = if off == u64::MAX {
            None
        } else {
            let off = usize::try_from(off).ok()?;
            if off.checked_add(blen)? > blobs_len {
                return None;
            }
            Some(Blob { map: map.clone(), off: HEADER + blobs_at + off, len: blen })
        };
        files.push(CachedFile { path, stamp, meta });
    }
    (r.i == blobs_at).then_some(files)
}

pub fn load(path: &Path) -> Option<(Vec<CachedFile>, Arc<Mapped>)> {
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() < HEADER as u64 {
        return None;
    }
    let map = Arc::new(Mapped::open(&file).ok()?);
    let files = decode(map.clone())?;
    Some((files, map))
}

pub fn save(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(b: &[u8]) -> Option<Vec<CachedFile>> {
        super::decode(Arc::new(Mapped::from_vec(b.to_vec())))
    }

    fn sample() -> NoteMeta {
        extract("---\ntags: [a]\nnested:\n  k: [1, true, null]\n---\n# H\n[[A#b|c]] [m](x.md) #t ^id\n\n- [x] t\n  - sub\n")
    }

    #[test]
    fn round_trip() {
        let plain = sample();
        let m = Meta::Owned(Arc::new(plain.clone()));
        let bytes = encode(
            [
                ("a/b.md", Stamp { mtime: -5, size: 9 }, Some(&m)),
                ("img.png", Stamp { mtime: 7, size: 1 }, None),
            ]
            .into_iter(),
        );
        let files = decode(&bytes).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a/b.md");
        assert_eq!(files[0].stamp, Stamp { mtime: -5, size: 9 });
        let blob = files[0].meta.clone().unwrap();
        assert_eq!(blob.decode().as_ref(), Some(&plain));
        let some = blob.decode_parts(parts::TAGS | parts::LIST_ITEMS).unwrap();
        assert_eq!((&some.tags, &some.list_items, some.words), (&plain.tags, &plain.list_items, plain.words));
        assert!(some.links.is_empty() && some.frontmatter.is_none() && some.headings.is_empty());
        let mut targets = Vec::new();
        blob.link_targets(&mut |t| targets.push(t.to_string()));
        assert_eq!(targets, plain.links.iter().map(|l| l.target.to_string()).collect::<Vec<_>>());
        assert!(files[1].meta.is_none());
        // Re-encoding copies cached blobs verbatim.
        let cached = Meta::Cached(blob);
        let again = encode(
            [
                ("a/b.md", Stamp { mtime: -5, size: 9 }, Some(&cached)),
                ("img.png", Stamp { mtime: 7, size: 1 }, None),
            ]
            .into_iter(),
        );
        assert_eq!(again, bytes);
    }

    #[test]
    fn corruption_is_detected_never_panics() {
        let m = Meta::Owned(Arc::new(sample()));
        let bytes = encode([("a.md", Stamp::default(), Some(&m))].into_iter());
        assert!(decode(&bytes[..bytes.len() - 1]).is_none(), "truncated");
        assert!(decode(&[]).is_none());
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0x5a;
            assert!(decode(&b).is_none(), "flip at {i} must be rejected");
        }
        // Even with a valid checksum over garbage, decoding must fail cleanly.
        let mut garbage = bytes.clone();
        for b in garbage[HEADER + 12..].iter_mut() {
            *b = b.wrapping_mul(31).wrapping_add(7);
        }
        let sum = fold64(&garbage[HEADER..]);
        garbage[16..24].copy_from_slice(&sum.to_le_bytes());
        for f in decode(&garbage).unwrap_or_default() {
            if let Some(b) = f.meta {
                let _ = b.decode();
                let _ = b.decode_parts(0);
                b.link_targets(&mut |_| {});
            }
        }
    }
}
