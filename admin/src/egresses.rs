//! The deployment's egresses: the upstream proxies a session may leave through, read once at startup from
//! `ADMIN_EGRESS_FILE` (it holds their credentials: owned by the admin's uid or root, and nothing for others), each with the time zone and languages its browsers then show (the operator states where the
//! upstream leaves; nothing here can check it). A malformed file stops the admin; its errors never quote the file.
//!
//! ```json
//! {"egresses": [{"name": "br-home", "proxy": "http://user-{session}:password@proxy.example.net:8080",
//!                "timezone": "America/Sao_Paulo", "languages": ["pt-BR", "pt", "en-US", "en"]}]}
//! ```

use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde::Deserialize;

use crate::app::Locale;

/// The largest file read, and the most egresses it may name.
const MAX_FILE: u64 = 64 * 1024;
const MAX_EGRESSES: usize = 32;

/// One egress a person may choose for a session.
pub struct Egress {
    pub name: String,
    pub upstream: egress::Upstream,
    pub locale: Locale,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    egresses: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    proxy: String,
    timezone: String,
    languages: Vec<String>,
}

/// Reads the file once: a regular file (not a link; opened without blocking, so a FIFO cannot stall the start) owned
/// by `uid` or root, which its group may at most read and others not at all, at most 64 KiB.
pub fn load(path: &Path, uid: u32) -> Result<Vec<Egress>, String> {
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags.bits() as i32)
        .open(path)
        .map_err(|e| format!("cannot open it: {}", e.kind()))?;
    let meta = file.metadata().map_err(|e| format!("cannot inspect it: {}", e.kind()))?;
    if !meta.is_file() {
        return Err("it is not a regular file".into());
    }
    if meta.uid() != uid && meta.uid() != 0 {
        return Err("it is owned by neither the admin's uid nor root".into());
    }
    if meta.mode() & 0o027 != 0 {
        return Err("its group may write it or others may use it".into());
    }
    let mut text = Vec::new();
    file.by_ref().take(MAX_FILE + 1).read_to_end(&mut text).map_err(|e| format!("cannot read it: {}", e.kind()))?;
    if text.len() as u64 > MAX_FILE {
        return Err("it is larger than 64 KiB".into());
    }
    parse(&text)
}

/// The egresses a file describes, checked as the controller and the session's proxy will check them again.
pub fn parse(text: &[u8]) -> Result<Vec<Egress>, String> {
    // Only the position of a JSON error: its message may quote the credentials.
    let file: File = serde_json::from_slice(text)
        .map_err(|e| format!("not the expected JSON (line {}, column {})", e.line(), e.column()))?;
    if file.egresses.len() > MAX_EGRESSES {
        return Err(format!("more than {MAX_EGRESSES} egresses"));
    }
    let mut egresses: Vec<Egress> = Vec::with_capacity(file.egresses.len());
    for (i, entry) in file.egresses.into_iter().enumerate() {
        let name = entry.name;
        let named = (1..=32).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !named {
            return Err(format!("egress {}: the name must be 1 to 32 of a-z, 0-9, and -", i + 1));
        }
        if egresses.iter().any(|e| e.name == name) {
            return Err(format!("egress {name}: the name is used twice"));
        }
        let upstream = egress::Upstream::parse_url(&entry.proxy).map_err(|why| format!("egress {name}: {why}"))?;
        // A session's id is 12 characters: the placeholder must still leave a valid user name.
        upstream.for_session("0123456789ab").map_err(|why| format!("egress {name}: {why}"))?;
        if !locale::valid_timezone(&entry.timezone) {
            return Err(format!("egress {name}: the time zone is not an IANA name such as America/Sao_Paulo"));
        }
        if !locale::valid_languages(&entry.languages) {
            return Err(format!("egress {name}: languages must be 1 to {} tags such as pt-BR", locale::MAX_LANGUAGES));
        }
        egresses.push(Egress {
            name,
            upstream,
            locale: Locale { timezone: entry.timezone, languages: entry.languages },
        });
    }
    Ok(egresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(entries: &str) -> String {
        format!(r#"{{"egresses": [{entries}]}}"#)
    }

    const BERLIN: &str = r#"{"name": "berlin", "proxy": "socks5://user-{session}:secret@proxy.example.net:1080",
                             "timezone": "Europe/Berlin", "languages": ["de-DE", "de"]}"#;

    #[test]
    fn a_file_names_its_egresses() {
        let egresses = parse(file(BERLIN).as_bytes()).unwrap();
        assert_eq!(egresses.len(), 1);
        assert_eq!(egresses[0].name, "berlin");
        assert_eq!(egresses[0].upstream.port, 1080);
        assert_eq!(egresses[0].locale.timezone, "Europe/Berlin");
        assert!(parse(file("").as_bytes()).unwrap().is_empty());
    }

    #[test]
    fn errors_never_quote_the_file() {
        let secretive = [
            file(&BERLIN.replace("socks5", "ftp")),
            file(&BERLIN.replace("\"berlin\"", "\"Berlin!\"")),
            file(&[BERLIN, BERLIN].join(",")),
            file(&BERLIN.replace("Europe/Berlin", "Europe/Nowhere")),
            file(&BERLIN.replace(r#"["de-DE", "de"]"#, "[]")),
            file(&BERLIN.replace(":1080", "")),
            file(&BERLIN.replace("\"timezone\"", "\"secret\": 1, \"timezone\"")),
            r#"{"egresses": "secret"}"#.to_owned(),
            "secret".to_owned(),
        ];
        for text in secretive {
            let error = match parse(text.as_bytes()) {
                Err(error) => error,
                Ok(_) => panic!("accepted: {text}"),
            };
            assert!(!error.contains("secret") && !error.contains("proxy.example"), "{error}");
        }
    }

    #[test]
    fn only_a_file_no_one_else_may_write_is_read() {
        let dir = crate::private::test_dir("egresses");
        let path = dir.join("egresses.json");
        std::fs::write(&path, file(BERLIN)).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let uid = rustix::process::getuid().as_raw();
        for (mode, ok) in [(0o400, true), (0o600, true), (0o640, true), (0o644, false), (0o660, false), (0o604, false)]
        {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(load(&path, uid).is_ok(), ok, "{mode:o}");
        }
        // Another owner (neither this uid nor root).
        assert!(load(&path, uid + 1).is_err() || uid == 0);
        let link = dir.join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(load(&link, uid).is_err());
        assert!(load(&dir, uid).is_err());
        // A FIFO is refused at once rather than read.
        let fifo = dir.join("fifo");
        rustix::fs::mknodat(rustix::fs::CWD, &fifo, rustix::fs::FileType::Fifo, 0o600.into(), 0).unwrap();
        assert!(load(&fifo, uid).is_err());
    }
}
