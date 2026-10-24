//! The state directory's private files (the password hash, the TLS key, the agent tokens): readable by the admin
//! only, and replaced atomically, so a crash or a full disk leaves the old contents or the new, never part of either.
//! The one public file there, the TLS certificate, is replaced the same way with its own mode.

use std::path::Path;

/// Writes `bytes` to `path`: a temporary file beside it (mode 0600), synced, then renamed over it, and the directory
/// synced so the rename itself survives a power loss before the next write: callers order several replacements.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    replace_with_mode(path, bytes, 0o600)
}

/// [`replace`], for a file of mode `mode` (whatever the umask).
pub fn replace_with_mode(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let temp = path.with_file_name(name);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        std::fs::File::open(path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?.sync_all()
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {e}", path.display())
    })
}

/// A new, empty state directory for a test, named after `prefix`: under the build's target directory, on disk, never in
/// a shared /tmp.
#[cfg(test)]
pub fn test_dir(prefix: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/tmp")
        .join(format!("admin-{prefix}-{}", crate::auth::random_token()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_replaced_whole_and_kept_private() {
        let dir = test_dir("private");
        let path = dir.join("password");
        std::fs::write(&path, "old").unwrap();
        replace(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left");
        assert!(replace(&dir.join("missing/password"), b"x").is_err());
    }
}
