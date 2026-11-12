//! Content shown inside notes that lives in other files: images and embedded notes.
//! Entries are keyed by vault path and reloaded when the file's modification time changes.

use std::collections::HashMap;
use std::sync::Arc;
use vault_core::VaultStore;
use walnut_ui::Image;

/// Files larger than this are not decoded.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// Decoded images are scaled down so that neither side exceeds this.
const MAX_IMAGE_SIDE: u32 = 2000;
const MAX_EMBED_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Default)]
pub struct Media {
    images: HashMap<String, (i64, Option<Image>)>,
    notes: HashMap<String, (i64, Option<Arc<str>>)>,
}

impl Media {
    /// The decoded image at a vault path. Only PNG is supported. Decoding happens on the calling
    /// thread the first time the image is shown.
    pub fn image(&mut self, store: &VaultStore, rel: &str) -> Option<Image> {
        let f = store.index.get(rel)?;
        if let Some((mtime, img)) = self.images.get(rel) {
            if *mtime == f.mtime {
                return img.clone();
            }
        }
        let img = (f.size <= MAX_IMAGE_BYTES)
            .then(|| std::fs::read(store.abs(&f.path)).ok())
            .flatten()
            .and_then(|bytes| Image::decode_png(&bytes, MAX_IMAGE_SIDE));
        self.images.insert(rel.to_string(), (f.mtime, img.clone()));
        img
    }

    /// The text of a note, for embedding.
    pub fn note(&mut self, store: &VaultStore, rel: &str) -> Option<Arc<str>> {
        let f = store.index.get(rel)?;
        if let Some((mtime, text)) = self.notes.get(rel) {
            if *mtime == f.mtime {
                return text.clone();
            }
        }
        let text: Option<Arc<str>> = (f.size <= MAX_EMBED_BYTES)
            .then(|| std::fs::read_to_string(store.abs(&f.path)).ok())
            .flatten()
            .map(Arc::from);
        self.notes.insert(rel.to_string(), (f.mtime, text.clone()));
        text
    }
}

/// The part of a note that `![[Note#subpath]]` embeds: the whole body, a heading's section, or
/// the block carrying `^id`. Front matter is never included.
pub fn embed_section<'a>(text: &'a str, meta: &vault_core::NoteMeta, subpath: &str) -> &'a str {
    let clamp = |a: u32, b: u32| {
        let (mut a, mut b) = ((a as usize).min(text.len()), (b as usize).min(text.len()));
        while !text.is_char_boundary(a) {
            a -= 1;
        }
        while !text.is_char_boundary(b) {
            b -= 1;
        }
        text[a..b.max(a)].trim_matches('\n')
    };
    let body = clamp(meta.frontmatter_end, text.len() as u32);
    let sub = subpath.trim();
    if sub.is_empty() {
        return body;
    }
    if let Some(id) = sub.strip_prefix('^') {
        return match meta.blocks.iter().find(|b| b.id.eq_ignore_ascii_case(id)) {
            Some(b) => clamp(b.start, b.end),
            None => body,
        };
    }
    // `A#B#C` addresses nested headings; the last one decides.
    let want = sub.rsplit('#').next().unwrap_or(sub).trim();
    let Some(i) = meta.headings.iter().position(|h| h.text.trim().eq_ignore_ascii_case(want)) else {
        return body;
    };
    let h = &meta.headings[i];
    let end =
        meta.headings[i + 1..].iter().find(|n| n.level <= h.level).map_or(text.len() as u32, |n| n.start);
    clamp(h.start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_sections() {
        let text = "---\na: 1\n---\n# One\nintro\n\n## Two\nbody ^blk\n\n### Deep\nd\n\n## Three\nlast\n";
        let meta = vault_core::meta::extract(text);
        assert!(embed_section(text, &meta, "").starts_with("# One"));
        assert_eq!(embed_section(text, &meta, "two"), "## Two\nbody ^blk\n\n### Deep\nd");
        assert_eq!(embed_section(text, &meta, "One#Three"), "## Three\nlast");
        assert_eq!(embed_section(text, &meta, "^blk"), "body ^blk");
        assert!(embed_section(text, &meta, "nope").starts_with("# One"));
    }
}
