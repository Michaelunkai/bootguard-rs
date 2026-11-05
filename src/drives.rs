//! Drive enumeration (fixed + removable/external, any filesystem) via plain Win32.

#[link(name = "kernel32")]
extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationW(root: *const u16, vol: *mut u16, vol_len: u32, serial: *mut u32, maxcomp: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
    fn GetTickCount64() -> u64;
}
#[link(name = "shell32")]
extern "system" {
    fn IsUserAnAdmin() -> i32;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub letter: char,
    /// 2 = removable (USB stick/SD), 3 = fixed (internal or external HDD/SSD)
    pub kind: u32,
    /// None when the volume is not ready / unformatted / unreadable
    pub fs: Option<String>,
}

impl Drive {
    pub fn kind_name(&self) -> &'static str {
        if self.kind == 2 { "removable/external" } else { "fixed" }
    }
    /// Filesystems the Windows boot-time checker (autochk) can verify and repair.
    pub fn checkable(&self) -> bool {
        matches!(self.fs.as_deref().map(|s| s.to_ascii_uppercase()).as_deref(), Some("NTFS") | Some("FAT") | Some("FAT32") | Some("EXFAT"))
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn fs_name(letter: char) -> Option<String> {
    let root = wide(&format!("{}:\\", letter));
    let mut buf = [0u16; 64];
    let ok = unsafe {
        GetVolumeInformationW(root.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32)
    };
    if ok == 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// Every local drive letter that is fixed or removable (network, CD and RAM drives are not disks to repair).
pub fn list() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut v = vec![];
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{}:\\", letter));
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        if kind == 2 || kind == 3 {
            v.push(Drive { letter, kind, fs: fs_name(letter) });
        }
    }
    v
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

pub fn uptime_secs() -> u64 {
    unsafe { GetTickCount64() / 1000 }
}
