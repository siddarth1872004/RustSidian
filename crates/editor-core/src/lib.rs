//! Editor model for Rustsidian. Headless: no rendering, no IO.

pub mod buffer;
pub mod deco;
pub mod editor;
pub mod fold;
pub mod table;
pub mod vim;

pub use buffer::{Buffer, Change, EditKind, Sel};
pub use editor::{Editor, Motion, Settings};
