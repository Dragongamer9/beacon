//! A Windows program's icon, read straight from the .exe (what icoextract does for Faugus Launcher): the PE
//! file's resource section has the icon group (RT_GROUP_ICON) and its images (RT_ICON); they're put back
//! together as an .ico, and the biggest image is saved as a PNG.
//!
//! Only the PE headers and the resource tree are read, every offset is bounds-checked, and the result goes
//! through the `ico` crate's decoder. A file that isn't a PE, or has no icon, is simply "no icon".

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
/// Programs bigger than this aren't read whole; their icon is skipped (installers can be several GB).
const MAX_FILE: u64 = 768 * 1024 * 1024;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

struct Section {
    va: u32,
    vsize: u32,
    raw: u32,
    raw_size: u32,
}

struct Pe<'a> {
    data: &'a [u8],
    sections: Vec<Section>,
    rsrc_rva: u32,
}

impl<'a> Pe<'a> {
    fn parse(data: &'a [u8]) -> Result<Pe<'a>> {
        if data.get(0..2) != Some(b"MZ") {
            bail!("not a Windows program");
        }
        let pe = u32_at(data, 0x3c).ok_or_else(|| anyhow!("truncated"))? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            bail!("not a PE file");
        }
        let coff = pe + 4;
        let nsec = u16_at(data, coff + 2).ok_or_else(|| anyhow!("truncated"))? as usize;
        let opt_size = u16_at(data, coff + 16).ok_or_else(|| anyhow!("truncated"))? as usize;
        let opt = coff + 20;
        let magic = u16_at(data, opt).ok_or_else(|| anyhow!("truncated"))?;
        let dirs = match magic {
            0x10b => opt + 96,
            0x20b => opt + 112,
            _ => bail!("unknown PE optional header"),
        };
        let ndirs = u32_at(data, dirs - 4).unwrap_or(0);
        if ndirs < 3 {
            bail!("no resources");
        }
        let rsrc_rva = u32_at(data, dirs + 16).ok_or_else(|| anyhow!("truncated"))?;
        if rsrc_rva == 0 {
            bail!("no resources");
        }
        let mut sections = Vec::new();
        let first = opt + opt_size;
        for i in 0..nsec.min(96) {
            let s = first + i * 40;
            sections.push(Section {
                vsize: u32_at(data, s + 8).ok_or_else(|| anyhow!("truncated"))?,
                va: u32_at(data, s + 12).ok_or_else(|| anyhow!("truncated"))?,
                raw_size: u32_at(data, s + 16).ok_or_else(|| anyhow!("truncated"))?,
                raw: u32_at(data, s + 20).ok_or_else(|| anyhow!("truncated"))?,
            });
        }
        Ok(Pe { data, sections, rsrc_rva })
    }

    fn offset(&self, rva: u32) -> Option<usize> {
        self.sections.iter().find_map(|s| {
            let size = s.vsize.max(s.raw_size);
            (rva >= s.va && rva < s.va.checked_add(size)?).then(|| (rva - s.va) as usize + s.raw as usize)
        })
    }

    fn bytes(&self, rva: u32, len: u32) -> Option<&'a [u8]> {
        let o = self.offset(rva)?;
        self.data.get(o..o.checked_add(len as usize)?)
    }

    /// The entries of the resource directory at `dir` (relative to the resource section): (id or None for a
    /// named entry, offset field).
    fn entries(&self, dir: u32) -> Vec<(Option<u32>, u32)> {
        let Some(base) = self.offset(self.rsrc_rva.saturating_add(dir)) else { return vec![] };
        let named = u16_at(self.data, base + 12).unwrap_or(0) as usize;
        let ids = u16_at(self.data, base + 14).unwrap_or(0) as usize;
        (0..(named + ids).min(4096))
            .filter_map(|i| {
                let e = base + 16 + i * 8;
                let name = u32_at(self.data, e)?;
                let off = u32_at(self.data, e + 4)?;
                Some(((name & 0x8000_0000 == 0).then_some(name), off))
            })
            .collect()
    }

    /// The first data leaf under a directory entry (the first language).
    fn leaf(&self, mut off: u32, depth: u32) -> Option<&'a [u8]> {
        for _ in 0..depth {
            if off & 0x8000_0000 == 0 {
                break;
            }
            off = self.entries(off & 0x7fff_ffff).first()?.1;
        }
        if off & 0x8000_0000 != 0 {
            return None;
        }
        let d = self.offset(self.rsrc_rva.saturating_add(off))?;
        let rva = u32_at(self.data, d)?;
        let size = u32_at(self.data, d + 4)?;
        self.bytes(rva, size)
    }

    /// Every resource of one type: (id, bytes); named ones get no id.
    fn of_type(&self, ty: u32) -> Vec<(Option<u32>, &'a [u8])> {
        let Some((_, off)) = self.entries(0).into_iter().find(|(id, _)| *id == Some(ty)) else { return vec![] };
        if off & 0x8000_0000 == 0 {
            return vec![];
        }
        self.entries(off & 0x7fff_ffff).into_iter().filter_map(|(id, o)| Some((id, self.leaf(o, 2)?))).collect()
    }
}

/// The program's main icon (its first icon group) as an .ico file.
pub fn ico_from_pe(data: &[u8]) -> Result<Vec<u8>> {
    let pe = Pe::parse(data)?;
    let icons: BTreeMap<u32, &[u8]> = pe.of_type(RT_ICON).into_iter().filter_map(|(id, b)| Some((id?, b))).collect();
    let groups = pe.of_type(RT_GROUP_ICON);
    let (_, grp) = groups.first().ok_or_else(|| anyhow!("the program has no icon"))?;
    let count = u16_at(grp, 4).ok_or_else(|| anyhow!("bad icon group"))? as usize;
    let mut entries: Vec<([u8; 8], &[u8])> = Vec::new();
    for i in 0..count.min(64) {
        let e = 6 + i * 14;
        let head: [u8; 8] = grp.get(e..e + 8).ok_or_else(|| anyhow!("bad icon group"))?.try_into().unwrap();
        let id = u16_at(grp, e + 12).ok_or_else(|| anyhow!("bad icon group"))? as u32;
        if let Some(img) = icons.get(&id) {
            entries.push((head, img));
        }
    }
    if entries.is_empty() {
        bail!("the program's icon has no images");
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * entries.len() as u32;
    for (head, img) in &entries {
        out.extend_from_slice(head);
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for (_, img) in &entries {
        out.extend_from_slice(img);
    }
    Ok(out)
}

/// The biggest image of an .ico as PNG bytes.
pub fn png_from_ico(ico: &[u8]) -> Result<Vec<u8>> {
    let dir = ico::IconDir::read(std::io::Cursor::new(ico)).context("reading the icon")?;
    let best = dir
        .entries()
        .iter()
        .max_by_key(|e| (e.width().max(e.height()), e.bits_per_pixel()))
        .ok_or_else(|| anyhow!("the icon is empty"))?;
    let img = best.decode().context("decoding the icon")?;
    let mut png = Vec::new();
    img.write_png(&mut png).context("writing the icon")?;
    Ok(png)
}

/// The icon of `exe` (a Windows program, an .ico or a .png) saved as a PNG at `out`. Ok(false) when the
/// file has no icon.
pub fn extract_to_png(exe: &Path, out: &Path) -> Result<bool> {
    let len = std::fs::metadata(exe).with_context(|| format!("reading {}", exe.display()))?.len();
    if len > MAX_FILE {
        return Ok(false);
    }
    let data = std::fs::read(exe).with_context(|| format!("reading {}", exe.display()))?;
    let png = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        data
    } else if data.starts_with(&[0, 0, 1, 0]) {
        png_from_ico(&data)?
    } else {
        match ico_from_pe(&data).and_then(|ico| png_from_ico(&ico)) {
            Ok(p) => p,
            Err(_) => return Ok(false),
        }
    };
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(out, png)?;
    Ok(true)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 2x2 32-bit icon image as Windows stores it in RT_ICON (a DIB: header, pixels, AND mask).
    fn dib() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes()); // biSize
        d.extend_from_slice(&2i32.to_le_bytes()); // width
        d.extend_from_slice(&4i32.to_le_bytes()); // height x2 (XOR + AND)
        d.extend_from_slice(&1u16.to_le_bytes()); // planes
        d.extend_from_slice(&32u16.to_le_bytes()); // bpp
        d.extend_from_slice(&[0u8; 24]); // compression .. clrImportant
        for _ in 0..4 {
            d.extend_from_slice(&[0x20, 0x80, 0xe0, 0xff]); // BGRA
        }
        d.extend_from_slice(&[0u8; 8]); // AND mask: 2 rows, padded to 4 bytes
        d
    }

    /// A minimal PE32+ with one .rsrc section holding RT_ICON #1 and RT_GROUP_ICON #1 pointing at it.
    pub(crate) fn tiny_exe() -> Vec<u8> {
        let img = dib();
        let rsrc_va = 0x1000u32;
        // Resource tree: root(2 ids) -> type dirs -> name dirs -> lang dirs -> data entries.
        let mut r: Vec<u8> = Vec::new();
        let dir = |r: &mut Vec<u8>, entries: &[(u32, u32)]| {
            r.extend_from_slice(&[0u8; 12]);
            r.extend_from_slice(&0u16.to_le_bytes());
            r.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for (id, off) in entries {
                r.extend_from_slice(&id.to_le_bytes());
                r.extend_from_slice(&off.to_le_bytes());
            }
        };
        const D: u32 = 0x8000_0000;
        // Layout (each dir with 1 entry is 24 bytes; root with 2 is 32).
        let root = 0u32;
        let t_icon = 32;
        let t_grp = t_icon + 24;
        let n_icon = t_grp + 24;
        let n_grp = n_icon + 24;
        let l_icon = n_grp + 24;
        let l_grp = l_icon + 24;
        let de_icon = l_grp + 24;
        let de_grp = de_icon + 16;
        let data_icon = de_grp + 16;
        let mut grp = Vec::new();
        grp.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
        grp.extend_from_slice(&[2, 2, 0, 0]);
        grp.extend_from_slice(&1u16.to_le_bytes());
        grp.extend_from_slice(&32u16.to_le_bytes());
        grp.extend_from_slice(&(img.len() as u32).to_le_bytes());
        grp.extend_from_slice(&1u16.to_le_bytes());
        let data_grp = data_icon + img.len() as u32;
        let _ = root;
        dir(&mut r, &[(RT_ICON, D | t_icon), (RT_GROUP_ICON, D | t_grp)]);
        dir(&mut r, &[(1, D | n_icon)]);
        dir(&mut r, &[(1, D | n_grp)]);
        dir(&mut r, &[(1033, D | l_icon)]);
        dir(&mut r, &[(1033, D | l_grp)]);
        dir(&mut r, &[(1033, de_icon)]);
        dir(&mut r, &[(1033, de_grp)]);
        for (off, len) in [(data_icon, img.len() as u32), (data_grp, grp.len() as u32)] {
            r.extend_from_slice(&(rsrc_va + off).to_le_bytes());
            r.extend_from_slice(&len.to_le_bytes());
            r.extend_from_slice(&[0u8; 8]);
        }
        assert_eq!(r.len() as u32, data_icon);
        r.extend_from_slice(&img);
        r.extend_from_slice(&grp);

        let mut f = vec![0u8; 0x200];
        f[0..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f[0x40..0x44].copy_from_slice(b"PE\0\0");
        let coff = 0x44;
        f[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        f[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        let opt_size = 112 + 16 * 8;
        f[coff + 16..coff + 18].copy_from_slice(&(opt_size as u16).to_le_bytes());
        let opt = coff + 20;
        f[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        f[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes
        let dirs = opt + 112;
        f[dirs + 16..dirs + 20].copy_from_slice(&rsrc_va.to_le_bytes());
        f[dirs + 20..dirs + 24].copy_from_slice(&(r.len() as u32).to_le_bytes());
        let sec = opt + opt_size;
        f[sec..sec + 8].copy_from_slice(b".rsrc\0\0\0");
        f[sec + 8..sec + 12].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[sec + 12..sec + 16].copy_from_slice(&rsrc_va.to_le_bytes());
        f[sec + 16..sec + 20].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
        f.extend_from_slice(&r);
        f
    }

    #[test]
    fn icon_out_of_a_program() {
        let exe = tiny_exe();
        let ico = ico_from_pe(&exe).unwrap();
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 1, 0]);
        let png = png_from_ico(&ico).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("game.exe");
        std::fs::write(&p, &exe).unwrap();
        let out = t.path().join("art/icon.png");
        assert!(extract_to_png(&p, &out).unwrap());
        assert!(std::fs::read(&out).unwrap().starts_with(b"\x89PNG"));
    }

    #[test]
    fn no_icon_is_not_an_error() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("x.exe");
        std::fs::write(&p, b"MZ not really").unwrap();
        assert!(!extract_to_png(&p, &t.path().join("i.png")).unwrap());
        assert!(ico_from_pe(b"hello").is_err());
        // Truncated headers never panic.
        let exe = tiny_exe();
        for n in [2, 0x40, 0x48, 0x60, 0x100, 0x210, 0x260] {
            let _ = ico_from_pe(&exe[..n]);
        }
    }
}
