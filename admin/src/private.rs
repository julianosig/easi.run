//! The state directory's private files (the password hash, the profiles' key salt, the second step, the agent tokens,
//! the profiles and egresses, the record of the certificate it made, and the TLS key it made): readable by the admin
//! only, and replaced (or created) atomically, so a crash or a full disk leaves the old contents or the new, never part
//! of either. All but the TLS key are read only as such (`read`); the key is read as it is, as it may be an operator's
//! own.
//! The one public file there, the TLS certificate, is replaced the same way with its own mode.

use std::path::Path;

/// What a read or a removal of a file gave, with a file that is not there as none rather than an error.
pub fn if_there<T>(done: std::io::Result<T>) -> std::io::Result<Option<T>> {
    match done {
        Ok(done) => Ok(Some(done)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The largest private file read.
pub const MAX_FILE: u64 = 256 * 1024;

/// The bytes of the private file at `path`, none when it is not there. It is read only if it is a regular file (not a
/// link; opened without blocking, so a FIFO cannot stall the start) owned by the admin's uid, which no one else may
/// use, at most [`MAX_FILE`]; one that cannot be read whole is refused, and the errors never quote it.
pub fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let shown = path.display();
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let opened = std::fs::OpenOptions::new().read(true).custom_flags(flags.bits() as i32).open(path);
    let mut file = match if_there(opened) {
        Ok(Some(file)) => file,
        Ok(None) => return Ok(None),
        Err(e) => return Err(format!("{shown}: cannot open it: {}", e.kind())),
    };
    let meta = file.metadata().map_err(|e| format!("{shown}: cannot inspect it: {}", e.kind()))?;
    if !meta.is_file() || meta.uid() != rustix::process::getuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err(format!("{shown}: not a regular file of the admin's alone (mode 0600)"));
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|e| format!("{shown}: {}", e.kind()))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(format!("{shown}: larger than {} KiB", MAX_FILE / 1024));
    }
    Ok(Some(bytes))
}

/// The private file `path`'s `bytes` as JSON; an error says only where, as its message may quote a secret.
pub fn json<T: serde::de::DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes)
        .map_err(|e| format!("{}: not the expected JSON (line {}, column {})", path.display(), e.line(), e.column()))
}

/// A replacement that failed, and whether the new contents are in place all the same: only their directory's sync
/// failed, so a power loss may still undo them.
#[derive(Debug)]
pub struct Unreplaced {
    pub why: String,
    pub in_place: bool,
}

impl std::fmt::Display for Unreplaced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.why)
    }
}

impl From<Unreplaced> for String {
    fn from(unreplaced: Unreplaced) -> String {
        unreplaced.why
    }
}

/// Writes `bytes` to `path`: a temporary file beside it (mode 0600), synced, then renamed over it, and the directory
/// synced so the rename itself survives a power loss before the next write: callers order several replacements.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<(), Unreplaced> {
    replace_with_mode(path, bytes, 0o600)
}

/// [`replace`], for a file of mode `mode` (whatever the umask).
pub fn replace_with_mode(path: &Path, bytes: &[u8], mode: u32) -> Result<(), Unreplaced> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let temp = path.with_file_name(name);
    let failed = |e: std::io::Error, in_place| Unreplaced { why: format!("{}: {e}", path.display()), in_place };
    let written = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(failed(e, false));
    }
    sync_parent(path).map_err(|e| failed(e, true))
}

/// Syncs the directory `path` is in, so a rename, link, or removal there survives a power loss.
pub fn sync_parent(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?.sync_all()
}

/// Writes `bytes` to `path` (mode 0600) only if nothing is there, atomically: a temporary file of its own beside it,
/// synced, then linked at `path`, which fails if anything is there already, even when another process writes it at the
/// same moment. `Ok(false)` when something was there: it is left as it was.
pub fn create(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", crate::auth::random_token()));
    let temp = path.with_file_name(name);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        match std::fs::hard_link(&temp, path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(e) => return Err(e),
        }
        sync_parent(path)?;
        Ok(true)
    })();
    let _ = std::fs::remove_file(&temp);
    written.map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod test_dir;
#[cfg(test)]
pub use test_dir::{TestDir, test_dir};

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

    #[test]
    fn a_file_is_created_only_where_none_is() {
        let dir = test_dir("private");
        let path = dir.join("password");
        assert_eq!(create(&path, b"first"), Ok(true));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(create(&path, b"second"), Ok(false));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first", "left as it was");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left");
        assert!(create(&dir.join("missing/password"), b"x").is_err());
    }
}
