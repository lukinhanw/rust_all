//! Client of the rust-search service (rs.exe --serve): one named pipe per indexed drive. Shared by rs, rs-launcher and topbar,
//! so asking for files never needs a new process.
use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
};
use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

pub fn pipe_name(d: char) -> String {
    format!("\\\\.\\pipe\\rs-search-{d}")
}

/// Letters of all mounted drives. Cheap: no volume is touched, so sleeping disks stay asleep.
pub fn drive_letters() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26).filter(|i| mask & (1 << i) != 0).map(|i| (b'A' + i as u8) as char).collect()
}

/// Asks drive `d`'s service. None = no service for that drive (pipe missing).
pub fn ask(d: char, q: &str) -> Option<io::Result<String>> {
    let mut p = match OpenOptions::new().read(true).write(true).open(pipe_name(d)) {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(e)),
    };
    Some((|| {
        p.write_all(q.as_bytes())?;
        let (mut out, mut chunk) = (Vec::new(), [0u8; 1 << 16]);
        loop {
            match p.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                // the server ends its reply by hanging up: BROKEN_PIPE (109) / NOT_CONNECTED (233) is EOF
                Err(e) if matches!(e.raw_os_error(), Some(109 | 233)) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    })())
}

/// Result paths from every drive's service (the "... N more" line dropped). Empty when no service runs.
pub fn ask_all(q: &str) -> Vec<String> {
    drive_letters()
        .into_iter()
        .filter_map(|d| ask(d, q)?.ok())
        .flat_map(|s| s.lines().filter(|l| !l.is_empty() && !l.starts_with("... ")).map(String::from).collect::<Vec<_>>())
        .collect()
}
