//! The only `unsafe` in this crate: mapping the index cache file read-only.
#![allow(unsafe_code)]

use std::fs::File;
use std::io;

pub struct Mapped(Inner);

enum Inner {
    File(memmap2::Mmap),
    Mem(Vec<u8>),
}

impl Mapped {
    pub fn open(file: &File) -> io::Result<Mapped> {
        // SAFETY: the mapping is read-only and private to this process, and it lives as long as
        // any note still refers to metadata inside it. The cache file is only ever replaced by an
        // atomic rename (never truncated or written in place by Rustwalnut), so the mapped bytes
        // stay valid: a replaced file keeps its inode until the map is dropped. If another
        // program truncated the file underneath us the process could receive SIGBUS; the content
        // is checksummed before use.
        let map = unsafe { memmap2::Mmap::map(file)? };
        Ok(Mapped(Inner::File(map)))
    }

    pub fn from_vec(bytes: Vec<u8>) -> Mapped {
        Mapped(Inner::Mem(bytes))
    }

    pub fn bytes(&self) -> &[u8] {
        match &self.0 {
            Inner::File(m) => m,
            Inner::Mem(v) => v,
        }
    }

    /// Drop the resident pages of the mapping. They are read back from the page cache on the
    /// next access, so this only trades a few page faults for a smaller resident set.
    pub fn release(&self) {
        #[cfg(unix)]
        if let Inner::File(m) = &self.0 {
            // SAFETY: the mapping is read-only and file-backed, so discarding its pages loses
            // nothing; later reads fault them in again from the file.
            let _ = unsafe { m.unchecked_advise(memmap2::UncheckedAdvice::DontNeed) };
        }
    }
}
