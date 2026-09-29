// SPDX-License-Identifier: GPL-3.0-or-later

//! Burning disc images to BD-R/BD-RE with xorriso (libburn), and
//! verifying the result by reading the disc back.

use anyhow::{bail, Context, Result};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

pub fn xorriso_bin() -> String {
    std::env::var("SPINDLE_XORRISO").unwrap_or_else(|_| "xorriso".into())
}

/// An optical drive.
#[derive(Debug, Clone, PartialEq)]
pub struct Drive {
    pub path: PathBuf,
    /// Vendor and model, e.g. "ASUS BW-16D1HT".
    pub name: String,
}

/// Optical drives on this computer.
pub fn drives() -> Vec<Drive> {
    let Ok(entries) = std::fs::read_dir("/sys/block") else { return vec![] };
    let mut out: Vec<Drive> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("sr"))
        .map(|e| {
            let dev = e.file_name().to_string_lossy().into_owned();
            let read = |f: &str| std::fs::read_to_string(e.path().join("device").join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
            let name = format!("{} {}", read("vendor"), read("model")).trim().to_string();
            Drive { path: PathBuf::from("/dev").join(dev), name: if name.is_empty() { gettextrs::gettext("Optical Drive") } else { name } }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A disc in a drive.
#[derive(Debug, Clone, PartialEq)]
pub struct Disc {
    /// "BD-R", "BD-RE", "DVD+R", …
    pub kind: String,
    pub blank: bool,
    /// Can be erased and written again.
    pub rewritable: bool,
    /// Bytes that can still be written.
    pub free: u64,
}

impl Disc {
    pub fn is_bluray(&self) -> bool {
        self.kind.starts_with("BD")
    }

}

#[derive(Debug, Clone, PartialEq)]
pub enum DiscState {
    NoDisc,
    /// Mounted or opened by another program.
    Busy,
    Ready(Disc),
}

/// "23.3g" → bytes.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let n: f64 = num.trim().parse().ok()?;
    let mult = match unit.to_ascii_lowercase().as_str() {
        "" | "s" => 2048.0,
        "k" => 1024.0,
        "m" => 1024.0 * 1024.0,
        "g" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// Parse `xorriso -toc` output.
fn parse_toc(out: &str) -> DiscState {
    let field = |key: &str| out.lines().find_map(|l| l.strip_prefix(key).map(|v| v.trim_start_matches([' ', ':']).trim().to_string()));
    if out.contains("busy device") {
        return DiscState::Busy;
    }
    let Some(kind) = field("Media current") else { return DiscState::NoDisc };
    if kind.is_empty() || kind.contains("is not recognizable") || kind.contains("none") {
        return DiscState::NoDisc;
    }
    let status = field("Media status").unwrap_or_default();
    let free = field("Media summary")
        .and_then(|s| s.split(',').find(|p| p.contains("free")).and_then(|p| parse_size(p.trim().trim_end_matches("free"))))
        .unwrap_or(0);
    let kind = kind.split_whitespace().next().unwrap_or("").trim_end_matches(',').to_string();
    DiscState::Ready(Disc {
        rewritable: kind.ends_with("RW") || kind.ends_with("RE") || kind == "DVD-RAM",
        blank: status.contains("is blank"),
        free,
        kind,
    })
}

/// What is in `drive`, without disturbing a disc that is in use.
pub fn check(drive: &Path) -> DiscState {
    let out = Command::new(xorriso_bin())
        .args(["-drive_access", "shared:readonly", "-indev"])
        .arg(drive)
        .arg("-toc")
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) => parse_toc(&format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(_) => DiscState::NoDisc,
    }
}

/// "  5 of  30 MB written" → fraction.
fn parse_progress(line: &str) -> Option<f64> {
    let i = line.find(" MB written")?;
    let head = &line[..i];
    let mut nums = head.split_whitespace().rev();
    let total: f64 = nums.next()?.parse().ok()?;
    let _of = nums.next()?;
    let done: f64 = nums.next()?.parse().ok()?;
    (total > 0.0).then(|| (done / total).clamp(0.0, 1.0))
}

/// Write `image` to the disc in `drive`, erasing a rewritable disc first.
/// Where the system has the disc in `drive` mounted (GNOME mounts discs
/// when they go in), asked of UDisks, which also works in the Flatpak.
fn udisks_block(drive: &Path) -> Option<String> {
    let name = drive.file_name()?.to_string_lossy().into_owned();
    Some(format!("/org/freedesktop/UDisks2/block_devices/{name}"))
}

fn mount_points(bus: &gio::DBusConnection, object: &str) -> Vec<String> {
    let reply = bus.call_sync(
        Some("org.freedesktop.UDisks2"),
        object,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&("org.freedesktop.UDisks2.Filesystem", "MountPoints").to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    );
    // No Filesystem interface: nothing on the disc to mount.
    let Ok(reply) = reply else { return vec![] };
    let Some(value) = reply.child_value(0).as_variant() else { return vec![] };
    let points: Vec<Vec<u8>> = value.get().unwrap_or_default();
    points.into_iter().map(|p| String::from_utf8_lossy(&p).trim_end_matches('\0').to_string()).collect()
}

/// Unmount the disc in `drive` if the system mounted it: a mounted disc
/// can't be burned ("busy").
pub fn unmount(drive: &Path) -> Result<()> {
    let Some(object) = udisks_block(drive).filter(|_| is_device(drive)) else { return Ok(()) };
    let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else { return Ok(()) };
    let points = mount_points(&bus, &object);
    if points.is_empty() {
        return Ok(());
    }
    let options: std::collections::HashMap<String, glib::Variant> = Default::default();
    bus.call_sync(
        Some("org.freedesktop.UDisks2"),
        &object,
        "org.freedesktop.UDisks2.Filesystem",
        "Unmount",
        Some(&(options,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        30_000,
        gio::Cancellable::NONE,
    )
    .map_err(|e| anyhow::anyhow!("The disc is open ({}) and couldn't be unmounted: {}. Eject it in Files, put it back in, and try again.", points.join(", "), e.message()))?;
    Ok(())
}

pub fn burn(image: &Path, drive: &Path, cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<()> {
    // A plain file stands in for a drive (for testing).
    let dev = if is_device(drive) { drive.display().to_string() } else { format!("stdio:{}", drive.display()) };
    unmount(drive)?;
    let mut child = Command::new(xorriso_bin())
        .args(["-as", "cdrecord", "-v", "-dao", "blank=as_needed"])
        .arg(format!("dev={dev}"))
        .arg(image)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run xorriso (needed for burning)")?;
    // Progress arrives on stderr, split by carriage returns.
    let stderr = child.stderr.take().expect("stderr");
    let mut log = String::new();
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_until(b'\r', &mut buf)?;
        if n == 0 {
            break;
        }
        for line in String::from_utf8_lossy(&buf).split('\n') {
            if let Some(f) = parse_progress(line) {
                progress(f);
            } else if !line.trim().is_empty() {
                log.push_str(line.trim_end_matches('\r'));
                log.push('\n');
            }
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("cancelled");
        }
    }
    let status = child.wait()?;
    if !status.success() {
        let tail: Vec<&str> = log.lines().filter(|l| l.contains("FAILURE") || l.contains("SORRY") || l.contains("FATAL")).collect();
        let msg = if tail.is_empty() { log.lines().rev().take(3).collect::<Vec<_>>().join("\n") } else { tail.join("\n") };
        if msg.contains("busy device") {
            bail!("The drive is in use by another program (a file manager, or another burn). Close it and try again.\n\n{msg}");
        }
        bail!("burning failed: {msg}");
    }
    progress(1.0);
    Ok(())
}

fn is_device(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path).is_ok_and(|m| m.file_type().is_block_device())
}

/// Open the drive tray.
pub fn eject(drive: &Path) {
    let _ = Command::new(xorriso_bin()).arg("-outdev").arg(drive).args(["-eject", "all"]).stdin(Stdio::null()).output();
}

/// Reads blocks straight from a drive with SCSI READ(10) commands, so
/// the kernel's idea of the disc size (stale right after burning) doesn't
/// matter.
mod sg {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    #[repr(C)]
    struct SgIoHdr {
        interface_id: i32,
        dxfer_direction: i32,
        cmd_len: u8,
        mx_sb_len: u8,
        iovec_count: u16,
        dxfer_len: u32,
        dxferp: *mut c_void,
        cmdp: *mut u8,
        sbp: *mut u8,
        timeout: u32,
        flags: u32,
        pack_id: i32,
        usr_ptr: *mut c_void,
        status: u8,
        masked_status: u8,
        msg_status: u8,
        sb_len_wr: u8,
        host_status: u16,
        driver_status: u16,
        resid: i32,
        duration: u32,
        info: u32,
    }

    const SG_IO: std::ffi::c_ulong = 0x2285;
    const SG_DXFER_FROM_DEV: i32 = -3;
    const O_NONBLOCK: i32 = 0o4000;

    unsafe extern "C" {
        fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
    }

    pub struct Reader {
        file: File,
    }

    impl Reader {
        pub fn open(path: &Path) -> std::io::Result<Self> {
            let file = std::fs::OpenOptions::new().read(true).custom_flags(O_NONBLOCK).open(path)?;
            Ok(Reader { file })
        }

        /// Read `buf.len() / 2048` blocks from `lba`.
        pub fn read(&self, lba: u32, buf: &mut [u8]) -> std::io::Result<()> {
            let blocks = (buf.len() / 2048) as u16;
            let mut cdb = [0x28u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            cdb[2..6].copy_from_slice(&lba.to_be_bytes());
            cdb[7..9].copy_from_slice(&blocks.to_be_bytes());
            let mut sense = [0u8; 32];
            let mut hdr = SgIoHdr {
                interface_id: 'S' as i32,
                dxfer_direction: SG_DXFER_FROM_DEV,
                cmd_len: cdb.len() as u8,
                mx_sb_len: sense.len() as u8,
                iovec_count: 0,
                dxfer_len: blocks as u32 * 2048,
                dxferp: buf.as_mut_ptr().cast(),
                cmdp: cdb.as_mut_ptr(),
                sbp: sense.as_mut_ptr(),
                timeout: 60_000,
                flags: 0,
                pack_id: 0,
                usr_ptr: std::ptr::null_mut(),
                status: 0,
                masked_status: 0,
                msg_status: 0,
                sb_len_wr: 0,
                host_status: 0,
                driver_status: 0,
                resid: 0,
                duration: 0,
                info: 0,
            };
            // SAFETY: hdr points at live buffers of the stated sizes for the
            // duration of the call.
            let r = unsafe { ioctl(self.file.as_raw_fd(), SG_IO, &mut hdr as *mut SgIoHdr) };
            if r < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if hdr.status != 0 || hdr.host_status != 0 || hdr.driver_status != 0 {
                let key = if hdr.sb_len_wr >= 3 { sense[2] & 0x0f } else { 0 };
                return Err(std::io::Error::other(format!("read error at block {lba} (sense key {key:#x})")));
            }
            Ok(())
        }
    }
}

/// Read the disc (or a stand-in image file) back and compare it with
/// `image`.
pub fn verify(image: &Path, drive: &Path, cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<()> {
    const CHUNK: usize = 32 * 2048;
    let len = std::fs::metadata(image)?.len();
    let mut img = std::fs::File::open(image).with_context(|| format!("reading {}", image.display()))?;
    let reader = if is_device(drive) { Some(sg::Reader::open(drive).with_context(|| format!("opening {}", drive.display()))?) } else { None };
    let mut file = if reader.is_none() { Some(std::fs::File::open(drive)?) } else { None };
    let (mut a, mut b) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    let mut pos = 0u64;
    while pos < len {
        let n = ((len - pos) as usize).min(CHUNK);
        // Whole blocks from the disc; the image is a whole number of blocks.
        let n_blocks = n.div_ceil(2048);
        img.read_exact(&mut a[..n])?;
        match (&reader, &mut file) {
            (Some(r), _) => r.read((pos / 2048) as u32, &mut b[..n_blocks * 2048])?,
            (None, Some(f)) => f.read_exact(&mut b[..n])?,
            _ => unreachable!(),
        }
        if a[..n] != b[..n] {
            bail!("the disc differs from the image at block {}", pos / 2048 + a[..n].iter().zip(&b[..n]).position(|(x, y)| x != y).unwrap_or(0) as u64 / 2048);
        }
        pos += n as u64;
        progress(pos as f64 / len as f64);
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toc() {
        let blank = "Drive current: -outdev '/dev/sr0'\nMedia current: BD-R sequential recording\nMedia status : is blank\nMedia summary: 0 sessions, 0 data blocks, 0 data, 23.3g free\n";
        match parse_toc(blank) {
            DiscState::Ready(d) => {
                assert_eq!(d.kind, "BD-R");
                assert!(d.blank && !d.rewritable && d.is_bluray());
                assert!((d.free as f64 / 1e9 - 25.0).abs() < 0.1, "{}", d.free);
            }
            other => panic!("{other:?}"),
        }
        let dvd = "Media current: DVD+R\nMedia status : is written , is closed\nMedia summary: 1 session, 2193664 data blocks, 4284m data,     0 free\n";
        assert!(matches!(parse_toc(dvd), DiscState::Ready(Disc { blank: false, rewritable: false, free: 0, .. })));
        let re = "Media current: BD-RE\nMedia status : is written , is appendable\nMedia summary: 1 session, 100 data blocks, 1m data, 22.9g free\n";
        assert!(matches!(parse_toc(re), DiscState::Ready(Disc { rewritable: true, .. })));
        assert_eq!(parse_toc("libburn : SORRY : Cannot open busy device '/dev/sr0'"), DiscState::Busy);
        assert_eq!(parse_toc("xorriso : NOTE : no media"), DiscState::NoDisc);
    }

    #[test]
    fn parses_progress() {
        assert_eq!(parse_progress("xorriso : UPDATE :    5 of   30 MB written (fifo  0%) [buf  50%]"), Some(5.0 / 30.0));
        assert_eq!(parse_progress("Beginning to write data track."), None);
    }
}
