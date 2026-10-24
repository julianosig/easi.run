//! `dpkg-prune PACKAGE...`: used once at image build. Removes each installed package's files and its records from the
//! dpkg database, without a shell, so even the shell, the core utilities, and dpkg itself can go. The package database
//! keeps describing exactly what remains, so the image's SBOM stays exact. Finally removes itself.

use std::collections::BTreeSet;
use std::path::PathBuf;

const INFO: &str = "/var/lib/dpkg/info";
const STATUS: &str = "/var/lib/dpkg/status";

/// Whether a file in the dpkg info directory is one of `package`'s records: exactly `<package>.<kind>` or
/// `<package>:<arch>.<kind>`, where the kind (`list`, `md5sums`, `postinst`, ...) has no dot. A package name may itself
/// contain dots, so `login` must not match `login.defs.list`, the record of another package.
fn is_record(name: &str, package: &str) -> bool {
    let Some(rest) = name.strip_prefix(package) else { return false };
    let kind = match rest.strip_prefix(':') {
        Some(arch_and_kind) => arch_and_kind.split_once('.').map(|(arch, kind)| (!arch.contains('.'), kind)),
        None => rest.strip_prefix('.').map(|kind| (true, kind)),
    };
    kind.is_some_and(|(arch_ok, kind)| arch_ok && !kind.is_empty() && !kind.contains('.'))
}

fn records(package: &str) -> Vec<PathBuf> {
    std::fs::read_dir(INFO)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| is_record(&entry.file_name().to_string_lossy(), package))
        .map(|entry| entry.path())
        .collect()
}

fn list(package: &str) -> Result<Vec<PathBuf>, String> {
    let path = records(package)
        .into_iter()
        .find(|p| p.extension().is_some_and(|e| e == "list"))
        .ok_or(format!("{package} is not installed"))?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
}

fn main() {
    let packages: Vec<String> = std::env::args().skip(1).collect();
    let mut directories = BTreeSet::new();
    for package in &packages {
        let paths = match list(package) {
            Ok(paths) => paths,
            Err(e) => {
                // Not installed in this base: nothing to remove.
                eprintln!("dpkg-prune: {e}");
                continue;
            }
        };
        for path in paths {
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => {
                    directories.insert(path);
                }
                Ok(_) => {
                    let _ = std::fs::remove_file(&path);
                }
                Err(_) => {}
            }
        }
        for record in records(package) {
            let _ = std::fs::remove_file(record);
        }
    }
    // Deepest first; a directory another package still uses is not empty and stays.
    for dir in directories.iter().rev() {
        let _ = std::fs::remove_dir(dir);
    }
    let status = std::fs::read_to_string(STATUS).expect("the dpkg status");
    let kept: Vec<&str> = status
        .split("\n\n")
        .filter(|stanza| {
            let name = stanza.lines().find_map(|l| l.strip_prefix("Package: ")).unwrap_or("");
            !packages.iter().any(|p| p == name)
        })
        .collect();
    std::fs::write(STATUS, kept.join("\n\n")).expect("the dpkg status is writable");
    let _ = std::fs::remove_file(format!("{STATUS}-old"));
    let _ = std::fs::remove_file(std::env::current_exe().expect("this program's path"));
    eprintln!("dpkg-prune: removed {} packages", packages.len());
}

#[cfg(test)]
mod tests {
    use super::is_record;

    #[test]
    fn records_belong_to_exactly_one_package() {
        assert!(is_record("login.list", "login"));
        assert!(is_record("libc6:amd64.md5sums", "libc6"));
        assert!(is_record("login.defs.list", "login.defs"));
        assert!(!is_record("login.defs.list", "login"), "another package's record");
        assert!(!is_record("loginx.list", "login"));
        assert!(!is_record("login.", "login"));
        assert!(!is_record("login:arm64", "login"));
    }
}
