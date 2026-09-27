// SPDX-License-Identifier: GPL-3.0-or-later

//! UDF 2.50 disc image writer (ECMA-167 3rd edition + OSTA UDF 2.50), the
//! file system of BD-ROM and recordable Blu-ray.
//!
//! Image layout (2048-byte sectors):
//! * 16–18: volume recognition sequence (BEA01, NSR03, TEA01)
//! * 32 / 48: main / reserve volume descriptor sequence
//! * 64: logical volume integrity sequence
//! * 256 and the last sector: anchor volume descriptor pointers
//! * from 272: the partition, holding
//!   * block 0 / 1: metadata file and metadata mirror file entries
//!   * the metadata area (file set descriptor, directories, file entries)
//!     and its mirror
//!   * file data, stream files aligned to 96 sectors (6144-byte aligned
//!     units and 64 KiB ECC blocks)
//!
//! The mirror sits before the file data so only zeros and the final anchor
//! follow the last file (7-Zip relies on that to find the image's end).
//!
//! File data is addressed in the physical partition (partition reference
//! 0), everything else in the metadata partition (reference 1).

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const SECTOR: u64 = 2048;
const MAIN_VDS: u32 = 32;
const RESERVE_VDS: u32 = 48;
const LVID: u32 = 64;
const ANCHOR: u32 = 256;
const PARTITION_START: u32 = 272;
/// Metadata allocation and alignment unit, in blocks.
const META_UNIT: u32 = 32;
/// Stream files start on multiples of this many sectors.
const STREAM_ALIGN: u64 = 96;
/// Largest extent length (a multiple of the block size below 2^30).
const MAX_EXTENT: u64 = (1 << 30) - SECTOR;
const UDF_REVISION: u16 = 0x0250;
/// Unique ids 1–15 are reserved.
const FIRST_UNIQUE_ID: u64 = 16;

const TAG_PVD: u16 = 1;
const TAG_AVDP: u16 = 2;
const TAG_IUVD: u16 = 4;
const TAG_PD: u16 = 5;
const TAG_LVD: u16 = 6;
const TAG_USD: u16 = 7;
const TAG_TD: u16 = 8;
const TAG_LVID: u16 = 9;
const TAG_FSD: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_EFE: u16 = 266;

const FILE_TYPE_DIRECTORY: u8 = 4;
const FILE_TYPE_FILE: u8 = 5;
const FILE_TYPE_METADATA: u8 = 250;
const FILE_TYPE_METADATA_MIRROR: u8 = 251;

const AD_SHORT: u16 = 0;
const AD_LONG: u16 = 1;

/// CRC-ITU-T (polynomial 0x1021, initial value 0) as used by descriptor tags.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// Little-endian descriptor builder.
#[derive(Default)]
struct Buf(Vec<u8>);

impl Buf {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    fn zeros(&mut self, n: usize) -> &mut Self {
        self.0.resize(self.0.len() + n, 0);
        self
    }
}

/// Fill in the 16-byte tag at the start of `d` (whose first 16 bytes are
/// reserved for it). The CRC covers the rest of the descriptor.
fn tag(id: u16, location: u32, mut d: Vec<u8>) -> Vec<u8> {
    let crc = crc16(&d[16..]);
    let crc_len = (d.len() - 16) as u16;
    let mut t = Buf::default();
    t.u16(id).u16(3).u8(0).u8(0).u16(1).u16(crc).u16(crc_len).u32(location);
    d[..16].copy_from_slice(&t.0);
    d[4] = d[..16].iter().enumerate().filter(|(i, _)| *i != 4).fold(0u8, |s, (_, b)| s.wrapping_add(*b));
    d
}

/// OSTA compressed Unicode: 8-bit when every character fits, else UTF-16BE.
fn cs0(s: &str) -> Vec<u8> {
    if s.chars().all(|c| (c as u32) < 256) {
        std::iter::once(8).chain(s.chars().map(|c| c as u8)).collect()
    } else {
        let mut v = vec![16];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    }
}

/// Fixed-length "dstring": compressed text, zero padded, length in the
/// last byte.
fn dstring(s: &str, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    if s.is_empty() {
        return out;
    }
    let mut enc = cs0(s);
    let step = if enc[0] == 16 { 2 } else { 1 };
    while enc.len() > len - 1 {
        enc.truncate(enc.len() - step);
    }
    out[..enc.len()].copy_from_slice(&enc);
    out[len - 1] = enc.len() as u8;
    out
}

fn charspec() -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[1..24].copy_from_slice(b"OSTA Compressed Unicode");
    v
}

/// Entity identifier ("regid") with an 8-byte suffix.
fn regid(id: &str, suffix: [u8; 8]) -> Vec<u8> {
    let mut v = vec![0u8; 32];
    v[1..1 + id.len()].copy_from_slice(id.as_bytes());
    v[24..].copy_from_slice(&suffix);
    v
}

/// Suffix for domain and UDF identifiers: UDF revision, then flags or OS.
fn udf_suffix() -> [u8; 8] {
    let r = UDF_REVISION.to_le_bytes();
    [r[0], r[1], 0, 0, 0, 0, 0, 0]
}

fn domain_id() -> Vec<u8> {
    regid("*OSTA UDF Compliant", udf_suffix())
}

fn implementation_id() -> Vec<u8> {
    // Suffix: OS class 4 (UNIX), OS identifier 5 (Linux).
    regid("*Spindle", [4, 5, 0, 0, 0, 0, 0, 0])
}

/// ECMA-167 timestamp (UTC).
#[derive(Clone, Copy)]
struct Timestamp([u8; 12]);

impl Timestamp {
    fn from_unix(secs: u64) -> Self {
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i16;
        let mut b = Buf::default();
        // Type 1 (local time) with a UTC offset of 0.
        b.u16(1 << 12).bytes(&year.to_le_bytes()).u8(month).u8(day);
        b.u8((rem / 3600) as u8).u8((rem / 60 % 60) as u8).u8((rem % 60) as u8).zeros(3);
        Timestamp(b.0.try_into().expect("12 bytes"))
    }
}

fn long_ad(len: u32, block: u32, partition: u16, unique_id: u32) -> Vec<u8> {
    let mut b = Buf::default();
    b.u32(len).u32(block).u16(partition).u16(0).u32(unique_id);
    b.0
}

fn pad_sector(mut d: Vec<u8>) -> Vec<u8> {
    let n = d.len().div_ceil(SECTOR as usize).max(1) * SECTOR as usize;
    d.resize(n, 0);
    d
}

/// A file or directory of the image.
struct Node {
    name: String,
    /// Source file (None for directories).
    source: Option<PathBuf>,
    size: u64,
    children: Vec<usize>,
    parent: usize,
    unique_id: u64,
    /// Metadata block of the file entry.
    icb: u32,
    /// Directories: metadata block and length of the FIDs.
    fid_block: u32,
    fid_bytes: u64,
    /// Files: partition block of the data.
    data_block: u32,
}

impl Node {
    fn is_dir(&self) -> bool {
        self.source.is_none()
    }
}

/// Gather `dir` into nodes; index 0 is the root. Entries are sorted by name.
fn scan(dir: &Path) -> Result<Vec<Node>> {
    let mut nodes = vec![Node {
        name: String::new(),
        source: None,
        size: 0,
        children: vec![],
        parent: 0,
        unique_id: 0,
        icb: 0,
        fid_block: 0,
        fid_bytes: 0,
        data_block: 0,
    }];
    fn walk(dir: &Path, me: usize, nodes: &mut Vec<Node>) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = e.metadata()?;
            let idx = nodes.len();
            nodes.push(Node {
                name,
                source: (!meta.is_dir()).then(|| e.path()),
                size: if meta.is_dir() { 0 } else { meta.len() },
                children: vec![],
                parent: me,
                unique_id: 0,
                icb: 0,
                fid_block: 0,
                fid_bytes: 0,
                data_block: 0,
            });
            nodes[me].children.push(idx);
            if meta.is_dir() {
                walk(&e.path(), idx, nodes)?;
            }
        }
        Ok(())
    }
    walk(dir, 0, &mut nodes)?;
    Ok(nodes)
}

fn fid_len(name: &str) -> u64 {
    let l_fi = if name.is_empty() { 0 } else { cs0(name).len() };
    (38 + l_fi as u64).div_ceil(4) * 4
}

fn fid(characteristics: u8, name: &str, icb: u32, unique_id: u64, location: u32) -> Vec<u8> {
    let ident = if name.is_empty() { vec![] } else { cs0(name) };
    let mut b = Buf::default();
    b.zeros(16).u16(1).u8(characteristics).u8(ident.len() as u8);
    b.bytes(&long_ad(SECTOR as u32, icb, 1, unique_id as u32)).u16(0).bytes(&ident);
    let len = b.0.len().div_ceil(4) * 4;
    b.0.resize(len, 0);
    tag(TAG_FID, location, b.0)
}

struct EntryInfo {
    file_type: u8,
    permissions: u32,
    link_count: u16,
    size: u64,
    blocks: u64,
    unique_id: u64,
    ad_type: u16,
    ads: Vec<u8>,
}

/// Extended file entry.
fn efe(e: &EntryInfo, time: Timestamp, location: u32) -> Vec<u8> {
    let mut b = Buf::default();
    b.zeros(16);
    // ICB tag: strategy 4, one entry.
    b.u32(0).u16(4).u16(0).u16(1).u8(0).u8(e.file_type).zeros(6).u16(e.ad_type);
    b.u32(u32::MAX).u32(u32::MAX).u32(e.permissions).u16(e.link_count).u8(0).u8(0).u32(0);
    b.u64(e.size).u64(e.size).u64(e.blocks);
    for _ in 0..4 {
        b.bytes(&time.0);
    }
    b.u32(1).u32(0).zeros(16).zeros(16);
    b.bytes(&implementation_id()).u64(e.unique_id).u32(0).u32(e.ads.len() as u32).bytes(&e.ads);
    tag(TAG_EFE, location, b.0)
}

/// Long allocation descriptors for `size` bytes of contiguous data.
fn data_extents(size: u64, mut block: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut left = size;
    while left > 0 {
        let len = left.min(MAX_EXTENT);
        out.extend(long_ad(len as u32, block, 0, 0));
        block += (len / SECTOR) as u32;
        left -= len;
    }
    out
}

/// Where everything goes.
struct Layout {
    /// Metadata area size in blocks (a multiple of META_UNIT).
    meta_blocks: u32,
    /// Partition blocks of the metadata area and its mirror.
    meta_start: u32,
    mirror_start: u32,
    partition_len: u32,
    total_sectors: u32,
    files: u32,
    dirs: u32,
    next_unique_id: u64,
}

fn plan(nodes: &mut [Node]) -> Result<Layout> {
    // Metadata blocks: 0 FSD, 1 terminator, then per directory its entry
    // and FIDs, per file its entry.
    let mut next = 2u32;
    let mut unique = FIRST_UNIQUE_ID;
    let (mut files, mut dirs) = (0, 0);
    for i in 0..nodes.len() {
        nodes[i].icb = next;
        next += 1;
        if i > 0 {
            nodes[i].unique_id = unique;
            unique += 1;
        }
        if nodes[i].is_dir() {
            dirs += 1;
            let bytes = fid_len("") + nodes[i].children.iter().map(|&c| fid_len(&nodes[c].name)).sum::<u64>();
            nodes[i].fid_block = next;
            nodes[i].fid_bytes = bytes;
            next += bytes.div_ceil(SECTOR) as u32;
        } else {
            files += 1;
        }
    }
    let meta_blocks = next.div_ceil(META_UNIT) * META_UNIT;
    let meta_start = META_UNIT;
    let mirror_start = meta_start + meta_blocks;
    let mut block = (mirror_start + meta_blocks) as u64;
    for n in nodes.iter_mut().filter(|n| !n.is_dir()) {
        let stream = n.name.to_ascii_lowercase().ends_with(".m2ts") || n.name.to_ascii_lowercase().ends_with(".ssif");
        if stream {
            let abs = PARTITION_START as u64 + block;
            block += abs.div_ceil(STREAM_ALIGN) * STREAM_ALIGN - abs;
        }
        n.data_block = u32::try_from(block).context("disc image too large")?;
        // Allocation descriptors must fit in the file entry's block.
        if 216 + n.size.div_ceil(MAX_EXTENT) * 16 > SECTOR {
            bail!("{} is too large for a disc image", n.name);
        }
        block += n.size.div_ceil(SECTOR);
    }
    let partition_len = u32::try_from(block).context("disc image too large")?;
    let total_sectors = PARTITION_START.checked_add(partition_len).and_then(|n| n.checked_add(1)).context("disc image too large")?;
    Ok(Layout { meta_blocks, meta_start, mirror_start, partition_len, total_sectors, files, dirs, next_unique_id: unique })
}

/// Metadata area contents (file set, directories, file entries).
fn metadata(nodes: &[Node], lay: &Layout, label: &str, time: Timestamp) -> Vec<u8> {
    let mut area = vec![0u8; lay.meta_blocks as usize * SECTOR as usize];
    let mut put = |block: u32, d: &[u8]| {
        let at = block as usize * SECTOR as usize;
        area[at..at + d.len()].copy_from_slice(d);
    };

    // File set descriptor, then a terminator.
    let mut b = Buf::default();
    b.zeros(16).bytes(&time.0).u16(3).u16(3).u32(1).u32(1).u32(0).u32(0);
    b.bytes(&charspec()).bytes(&dstring(label, 128)).bytes(&charspec()).bytes(&dstring(label, 32));
    b.zeros(32).zeros(32);
    b.bytes(&long_ad(SECTOR as u32, nodes[0].icb, 1, 0)).bytes(&domain_id()).zeros(16).zeros(16).zeros(32);
    put(0, &tag(TAG_FSD, 0, b.0));
    put(1, &tag(TAG_TD, 1, vec![0u8; 512]));

    for n in nodes {
        let entry = if n.is_dir() {
            let subdirs = n.children.iter().filter(|&&c| nodes[c].is_dir()).count() as u16;
            // FIDs: parent first, then children; each tagged with the block it starts in.
            let mut fids = Vec::new();
            let parent = &nodes[n.parent];
            let block_of = |offset: usize| n.fid_block + (offset / SECTOR as usize) as u32;
            fids.extend(fid(0x0A, "", parent.icb, parent.unique_id, block_of(0)));
            for &c in &n.children {
                let child = &nodes[c];
                let chars = if child.is_dir() { 0x02 } else { 0x00 };
                let d = fid(chars, &child.name, child.icb, child.unique_id, block_of(fids.len()));
                fids.extend(d);
            }
            debug_assert_eq!(fids.len() as u64, n.fid_bytes);
            put(n.fid_block, &fids);
            let mut ad = Buf::default();
            ad.u32(n.fid_bytes as u32).u32(n.fid_block);
            EntryInfo {
                file_type: FILE_TYPE_DIRECTORY,
                permissions: 0x2529,
                link_count: 1 + subdirs,
                size: n.fid_bytes,
                blocks: n.fid_bytes.div_ceil(SECTOR),
                unique_id: n.unique_id,
                ad_type: AD_SHORT,
                ads: ad.0,
            }
        } else {
            EntryInfo {
                file_type: FILE_TYPE_FILE,
                permissions: 0x2108,
                link_count: 1,
                size: n.size,
                blocks: n.size.div_ceil(SECTOR),
                unique_id: n.unique_id,
                ad_type: AD_LONG,
                ads: data_extents(n.size, n.data_block),
            }
        };
        put(n.icb, &efe(&entry, time, n.icb));
    }
    area
}

fn metadata_file_entry(file_type: u8, start: u32, lay: &Layout, time: Timestamp, location: u32) -> Vec<u8> {
    let size = lay.meta_blocks as u64 * SECTOR;
    let mut ad = Buf::default();
    ad.u32(size as u32).u32(start);
    efe(
        &EntryInfo {
            file_type,
            permissions: 0,
            link_count: 1,
            size,
            blocks: lay.meta_blocks as u64,
            unique_id: 0,
            ad_type: AD_SHORT,
            ads: ad.0,
        },
        time,
        location,
    )
}

fn volume_descriptors(lay: &Layout, label: &str, set_id: &str, time: Timestamp, start: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut seq = 0u32;
    let mut next_seq = || {
        seq += 1;
        seq
    };

    // Primary volume descriptor.
    let mut b = Buf::default();
    b.zeros(16).u32(next_seq()).u32(0).bytes(&dstring(label, 32)).u16(1).u16(1).u16(2).u16(3).u32(1).u32(1);
    b.bytes(&dstring(set_id, 128)).bytes(&charspec()).bytes(&charspec()).zeros(8).zeros(8);
    b.zeros(32).bytes(&time.0).bytes(&implementation_id()).zeros(64).u32(0).u16(0).zeros(22);
    out.extend(pad_sector(tag(TAG_PVD, start, b.0)));

    // Implementation use volume descriptor with logical volume information.
    let mut b = Buf::default();
    b.zeros(16).u32(next_seq()).bytes(&regid("*UDF LV Info", udf_suffix()));
    b.bytes(&charspec()).bytes(&dstring(label, 128)).zeros(36 * 3).bytes(&implementation_id()).zeros(128);
    out.extend(pad_sector(tag(TAG_IUVD, start + 1, b.0)));

    // Partition descriptor.
    let mut b = Buf::default();
    b.zeros(16).u32(next_seq()).u16(1).u16(0).bytes(&regid("+NSR03", [0; 8])).zeros(128);
    b.u32(1).u32(PARTITION_START).u32(lay.partition_len).bytes(&implementation_id()).zeros(128).zeros(156);
    out.extend(pad_sector(tag(TAG_PD, start + 2, b.0)));

    // Logical volume descriptor: a physical and a metadata partition map.
    let mut b = Buf::default();
    b.zeros(16).u32(next_seq()).bytes(&charspec()).bytes(&dstring(label, 128)).u32(SECTOR as u32).bytes(&domain_id());
    b.bytes(&long_ad(SECTOR as u32, 0, 1, 0)).u32(6 + 64).u32(2).bytes(&implementation_id()).zeros(128);
    b.u32(2 * SECTOR as u32).u32(LVID);
    b.u8(1).u8(6).u16(1).u16(0);
    b.u8(2).u8(64).zeros(2).bytes(&regid("*UDF Metadata Partition", udf_suffix())).u16(1).u16(0);
    b.u32(0).u32(1).u32(u32::MAX).u32(META_UNIT).u16(META_UNIT as u16).u8(0).zeros(5);
    out.extend(pad_sector(tag(TAG_LVD, start + 3, b.0)));

    // Unallocated space descriptor (none) and terminator.
    let mut b = Buf::default();
    b.zeros(16).u32(next_seq()).u32(0);
    out.extend(pad_sector(tag(TAG_USD, start + 4, b.0)));
    out.extend(pad_sector(tag(TAG_TD, start + 5, vec![0u8; 512])));
    out
}

fn integrity_descriptor(lay: &Layout, time: Timestamp) -> Vec<u8> {
    let mut b = Buf::default();
    b.zeros(16).bytes(&time.0).u32(1).zeros(8).u64(lay.next_unique_id).zeros(24);
    b.u32(2).u32(46);
    // Free space, then size, of both partitions.
    b.u32(0).u32(0).u32(lay.partition_len).u32(lay.meta_blocks);
    b.bytes(&implementation_id()).u32(lay.files).u32(lay.dirs).u16(UDF_REVISION).u16(UDF_REVISION).u16(UDF_REVISION);
    let mut out = pad_sector(tag(TAG_LVID, LVID, b.0));
    out.extend(pad_sector(tag(TAG_TD, LVID + 1, vec![0u8; 512])));
    out
}

fn anchor(location: u32) -> Vec<u8> {
    let mut b = Buf::default();
    b.zeros(16).u32(16 * SECTOR as u32).u32(MAIN_VDS).u32(16 * SECTOR as u32).u32(RESERVE_VDS).zeros(480);
    pad_sector(tag(TAG_AVDP, location, b.0))
}

/// Volume labels: at most 30 characters, printable.
fn clean_label(label: &str) -> String {
    let l: String = label.chars().filter(|c| !c.is_control()).take(30).collect();
    if l.trim().is_empty() { "BLURAY".into() } else { l }
}

struct Out {
    w: BufWriter<File>,
    written: u64,
}

impl Out {
    fn write(&mut self, d: &[u8]) -> Result<()> {
        self.w.write_all(d)?;
        self.written += d.len() as u64;
        Ok(())
    }

    /// Zero-fill up to `sector`.
    fn seek_sector(&mut self, sector: u64) -> Result<()> {
        let target = sector * SECTOR;
        if target < self.written {
            bail!("UDF layout overlap at sector {sector}");
        }
        static ZEROS: [u8; 65536] = [0; 65536];
        while self.written < target {
            let n = (target - self.written).min(ZEROS.len() as u64) as usize;
            self.write(&ZEROS[..n])?;
        }
        Ok(())
    }
}

/// Write the contents of `dir` as a UDF 2.50 image to `iso`. `progress`
/// gets the fraction done and returns false to cancel.
pub fn write_image(dir: &Path, iso: &Path, label: &str, mut progress: impl FnMut(f64) -> bool) -> Result<()> {
    let mut nodes = scan(dir)?;
    let lay = plan(&mut nodes)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let time = Timestamp::from_unix(now);
    let label = clean_label(label);
    // Volume set identifiers start with 16 unique hex digits.
    let set_id = format!("{now:016X}{}", label);

    let file = File::create(iso).with_context(|| format!("creating {}", iso.display()))?;
    let mut out = Out { w: BufWriter::with_capacity(1 << 20, file), written: 0 };
    out.seek_sector(16)?;
    for id in [b"BEA01", b"NSR03", b"TEA01"] {
        let mut s = vec![0u8; SECTOR as usize];
        s[1..6].copy_from_slice(id);
        s[6] = 1;
        out.write(&s)?;
    }
    for start in [MAIN_VDS, RESERVE_VDS] {
        out.seek_sector(start as u64)?;
        out.write(&volume_descriptors(&lay, &label, &set_id, time, start))?;
    }
    out.seek_sector(LVID as u64)?;
    out.write(&integrity_descriptor(&lay, time))?;
    out.seek_sector(ANCHOR as u64)?;
    out.write(&anchor(ANCHOR))?;

    let part = |block: u32| PARTITION_START as u64 + block as u64;
    out.seek_sector(part(0))?;
    out.write(&pad_sector(metadata_file_entry(FILE_TYPE_METADATA, lay.meta_start, &lay, time, 0)))?;
    out.write(&pad_sector(metadata_file_entry(FILE_TYPE_METADATA_MIRROR, lay.mirror_start, &lay, time, 1)))?;
    let meta = metadata(&nodes, &lay, &label, time);
    out.seek_sector(part(lay.meta_start))?;
    out.write(&meta)?;
    out.write(&meta)?;

    let total: u64 = nodes.iter().map(|n| n.size).sum::<u64>().max(1);
    let mut done = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    for n in nodes.iter().filter(|n| !n.is_dir()) {
        out.seek_sector(part(n.data_block))?;
        let src = n.source.as_ref().expect("file");
        let mut f = File::open(src).with_context(|| format!("reading {}", src.display()))?;
        let mut left = n.size;
        while left > 0 {
            let want = left.min(buf.len() as u64) as usize;
            f.read_exact(&mut buf[..want]).with_context(|| format!("reading {}", src.display()))?;
            out.write(&buf[..want])?;
            left -= want as u64;
            done += want as u64;
            if !progress(done as f64 / total as f64) {
                bail!("cancelled");
            }
        }
    }
    out.seek_sector(lay.total_sectors as u64 - 1)?;
    out.write(&anchor(lay.total_sectors - 1))?;
    out.w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_ecma_example() {
        // ECMA-167 7.2.6 example: CRC of bytes 70 6A 77 is 0x3299.
        assert_eq!(crc16(&[0x70, 0x6A, 0x77]), 0x3299);
    }

    #[test]
    fn dates() {
        let t = Timestamp::from_unix(1_790_000_000).0;
        // 2026-09-21 14:13:20 UTC
        assert_eq!(i16::from_le_bytes([t[2], t[3]]), 2026);
        assert_eq!(&t[4..10], &[9, 21, 14, 13, 20, 0]);
    }

    #[test]
    fn tag_checksum_and_strings() {
        let d = tag(TAG_TD, 5, vec![0u8; 512]);
        let sum = d[..16].iter().enumerate().filter(|(i, _)| *i != 4).fold(0u8, |s, (_, b)| s.wrapping_add(*b));
        assert_eq!(d[4], sum);
        assert_eq!(u32::from_le_bytes(d[12..16].try_into().unwrap()), 5);
        let s = dstring("BDMV", 32);
        assert_eq!(&s[..5], b"\x08BDMV");
        assert_eq!(s[31], 5);
        assert_eq!(fid_len("index.bdmv"), 52);
    }

    #[test]
    fn writes_image() {
        let root = std::env::temp_dir().join(format!("spindle-udf-{}", crate::model::new_id()));
        let src = root.join("src");
        std::fs::create_dir_all(src.join("BDMV/STREAM")).unwrap();
        std::fs::write(src.join("BDMV/index.bdmv"), b"INDX0200").unwrap();
        std::fs::write(src.join("BDMV/STREAM/00001.m2ts"), vec![0x47u8; 6144 * 3]).unwrap();
        let iso = root.join("disc.iso");
        write_image(&src, &iso, "TEST DISC", |_| true).unwrap();
        let img = std::fs::read(&iso).unwrap();
        assert_eq!(img.len() % SECTOR as usize, 0);
        assert_eq!(&img[16 * 2048 + 1..16 * 2048 + 6], b"BEA01");
        let last = img.len() - SECTOR as usize;
        assert_eq!(u16::from_le_bytes([img[last], img[last + 1]]), TAG_AVDP);
        // The stream file starts on an aligned sector and holds the data.
        let pos = img.windows(4).position(|w| w == [0x47; 4]).unwrap();
        assert_eq!(pos as u64 % (STREAM_ALIGN * SECTOR), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
