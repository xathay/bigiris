//! Filesystem helpers that refuse to follow symlinks at the final path
//! component. Used by every per-file-manager installer that writes into
//! `/usr/share/...` (or `~/.local/share/...`): if an attacker plants a
//! symlink at one of our target names before `sudo --system` runs, plain
//! `std::fs::write` would silently rewrite whatever the symlink points
//! at — typically root-owned files outside our integration scope.
//!
//! Atomicity comes from the kernel: `open(O_NOFOLLOW)` returns `ELOOP`
//! when the last path component is a symlink, in a single syscall. No
//! TOCTOU window between "is it a symlink?" and "open it".

use std::io::{self, Write};
use std::path::Path;

/// `O_NOFOLLOW` on Linux. Hard-coded so we don't pull a direct `libc`
/// dep just for this one constant; the value is stable across every
/// current Linux kernel (see `include/uapi/asm-generic/fcntl.h`).
#[cfg(target_os = "linux")]
const O_NOFOLLOW: i32 = 0o400000;

/// Write `content` to `path`, creating or truncating, but refusing to
/// follow a symlink at the target. On non-Linux falls back to plain
/// `std::fs::write` (the project is Linux-only; this shim just keeps
/// `cargo check` happy on macOS dev boxes).
#[cfg(target_os = "linux")]
pub(crate) fn safe_write(path: &Path, content: impl AsRef<[u8]>) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;
    f.write_all(content.as_ref())?;
    f.flush()?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn safe_write(path: &Path, content: impl AsRef<[u8]>) -> io::Result<()> {
    std::fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_to_fresh_path() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("a.txt");
        safe_write(&p, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");
    }

    #[test]
    fn truncates_existing_regular_file() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("a.txt");
        std::fs::write(&p, "first content here").unwrap();
        safe_write(&p, "shorter").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "shorter");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn refuses_to_follow_symlink_at_target() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real.txt");
        let link = tmp.path().join("link.txt");
        std::fs::write(&real, "untouched").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let err = safe_write(&link, "attacker payload").unwrap_err();
        // Linux open(O_NOFOLLOW) on a symlink → ELOOP (errno 40). The
        // ErrorKind variant `FilesystemLoop` exists but is still
        // unstable in 1.94, so check raw_os_error instead. What
        // matters is that we did NOT clobber the file behind the link.
        assert_eq!(err.raw_os_error(), Some(40), "expected ELOOP, got {err:?}");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "untouched");
    }
}
