//! Embeds the application icon into the Windows executable. The resource file is written here
//! byte by byte, so no resource compiler is needed.

#[path = "src/icon.rs"]
mod icon;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];

/// One resource entry of a `.res` file, with numeric type and name.
fn entry(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, lang: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    for id in [kind, name] {
        out.extend(0xFFFFu16.to_le_bytes());
        out.extend(id.to_le_bytes());
    }
    out.extend(0u32.to_le_bytes());
    out.extend(flags.to_le_bytes());
    out.extend(lang.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(data);
    out.resize(out.len().next_multiple_of(4), 0);
}

/// A 32-bit icon image: bitmap header, bottom-up BGRA pixels, then an empty AND mask.
fn dib(size: u32) -> Vec<u8> {
    let rgba = icon::rgba(size);
    let mut out = Vec::new();
    out.extend(40u32.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend((size * 2).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend([0u8; 24]);
    for y in (0..size).rev() {
        for x in 0..size {
            let p = ((y * size + x) * 4) as usize;
            out.extend([rgba[p + 2], rgba[p + 1], rgba[p], rgba[p + 3]]);
        }
    }
    let mask_row = size.div_ceil(32) * 4;
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") != "windows" || var("CARGO_CFG_TARGET_ENV") != "msvc" {
        return;
    }
    let mut res = Vec::new();
    entry(&mut res, 0, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((SIZES.len() as u16).to_le_bytes());
    for (i, size) in SIZES.into_iter().enumerate() {
        let image = dib(size);
        let id = i as u16 + 1;
        // 256 is stored as 0 in the one-byte width and height.
        group.extend([size as u8, size as u8, 0, 0]);
        group.extend(1u16.to_le_bytes());
        group.extend(32u16.to_le_bytes());
        group.extend((image.len() as u32).to_le_bytes());
        group.extend(id.to_le_bytes());
        entry(&mut res, RT_ICON, id, 0x1010, 0x0409, &image);
    }
    entry(&mut res, RT_GROUP_ICON, 1, 0x1030, 0x0409, &group);
    let path = std::path::Path::new(&var("OUT_DIR")).join("rustsidian-icon.res");
    if std::fs::write(&path, res).is_ok() {
        println!("cargo:rustc-link-arg-bins={}", path.display());
    }
}
