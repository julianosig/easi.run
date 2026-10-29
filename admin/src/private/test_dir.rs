//! A test's state directory, for the admin's unit tests and its subcommands' (`tests/cli.rs`, which includes this file).

use std::path::{Path, PathBuf};

/// A new, empty state directory for a test, named after `prefix`: in the system's temporary directory, created afresh
/// (never one that is there already) and private to the test's user, and removed with all it holds when dropped.
pub fn test_dir(prefix: &str) -> TestDir {
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("admin-{prefix}-{:032x}", rand::random::<u128>()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    TestDir(dir)
}

/// A test's state directory (see [`test_dir`]).
pub struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
