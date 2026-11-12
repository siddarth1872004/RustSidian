//! Vault model for Rustsidian.

pub mod cache;
pub mod config;
pub mod fsio;
pub mod meta;
mod mmap;
pub mod path;
pub mod store;
pub mod vault;
pub mod watch;

pub use fsio::{Stamp, TrashMode};
pub use meta::NoteMeta;
pub use path::FileKind;
pub use store::{Change, OpenStats, VaultStore, WriteError};
pub use vault::{
    apply_edits, FileEdits, FileEntry, FileId, LinkFormat, LinkGraph, RenamePlan, TextEdit, Vault,
};
