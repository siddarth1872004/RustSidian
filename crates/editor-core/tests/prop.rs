use editor_core::{Buffer, Change, EditKind};
use proptest::prelude::*;

const FRAG: &[&str] = &[
    "\n",
    "\n\n",
    "# ",
    "- ",
    "> ",
    "```\n",
    "**",
    "[[",
    "]]",
    "$$",
    "%%",
    "---\n",
    "word ",
    "é",
    "| a |\n|---|\n",
    "1. ",
];

proptest! {
    #![proptest_config(ProptestConfig { cases: 1500, ..ProptestConfig::default() })]

    #[test]
    fn edits_keep_tree_in_sync_and_undo_restores(
        base in prop::collection::vec(prop::sample::select(FRAG), 0..40),
        edits in prop::collection::vec((0usize..2000, 0usize..12, prop::sample::select(FRAG), any::<bool>()), 1..20),
    ) {
        let original = base.concat();
        let mut b = Buffer::new(&original);
        let mut time = 0;
        for (at, del, ins, typing) in edits {
            let start = at % (b.len() + 1);
            let end = (start + del).min(b.len());
            time += if typing { 50 } else { 5000 };
            let kind = if typing { EditKind::Typing } else { EditKind::Other };
            b.apply(vec![Change { start, end, text: ins.to_string() }], None, kind, time);
            let text = b.text();
            prop_assert_eq!(b.doc(), &md_engine::parse(&text));
            for s in &b.sels {
                prop_assert!(s.end() <= text.len() && text.is_char_boundary(s.head) && text.is_char_boundary(s.anchor));
            }
        }
        let edited = b.text();
        while b.undo() {}
        prop_assert_eq!(b.text(), original.clone());
        prop_assert_eq!(b.doc(), &md_engine::parse(&original));
        while b.redo() {}
        prop_assert_eq!(b.text(), edited);
    }
}
