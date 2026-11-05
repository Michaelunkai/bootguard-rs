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
    fn ShellExecuteW(hwnd: isize, verb: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
}

/// Path as a plain Win32 string: drops a `\\?\` verbatim prefix that current_exe may return.
pub fn plain_path(p: &str) -> String {
    p.strip_prefix("\\\\?\\").unwrap_or(p).to_string()
}

/// Re-launch this exe elevated (UAC prompt) with the given arguments. True when Windows accepted the request.
/// The exe path is its own ShellExecute parameter, so spaces in it need no quoting.
pub fn relaunch_elevated(args: &str) -> bool {
    let exe = plain_path(&std::env::current_exe().unwrap_or_default().display().to_string());
    let wdir = std::path::Path::new(&exe).parent().map(|d| wide(&d.display().to_string()));
    let dptr = wdir.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
    let r = unsafe { ShellExecuteW(0, wide("runas").as_ptr(), wide(&exe).as_ptr(), wide(args).as_ptr(), dptr, 1) };
    r > 32
}

#[cfg(test)]
mod path_tests {
    use super::plain_path;
    #[test]
    fn strips_verbatim_prefix_and_keeps_spaces() {
        assert_eq!(plain_path("\\\\?\\C:\\Program Files\\bg\\bootguard.exe"), "C:\\Program Files\\bg\\bootguard.exe");
        assert_eq!(plain_path("C:\\a b\\x.exe"), "C:\\a b\\x.exe");
    }
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
