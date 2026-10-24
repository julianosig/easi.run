//! The images a session runs: each pinned by an immutable id or digest, never by a tag that could move.

/// Digest-pinned images, from `IMAGE_DISPLAY`, `IMAGE_BROWSER`, `IMAGE_AGENT`, and `IMAGE_PROXY`.
#[derive(Clone, Debug)]
pub struct Images {
    pub display: String,
    pub browser: String,
    pub agent: String,
    pub proxy: String,
}

impl Images {
    pub fn new(display: &str, browser: &str, agent: &str, proxy: &str) -> Result<Self, String> {
        Ok(Self {
            display: pinned("IMAGE_DISPLAY", display)?,
            browser: pinned("IMAGE_BROWSER", browser)?,
            agent: pinned("IMAGE_AGENT", agent)?,
            proxy: pinned("IMAGE_PROXY", proxy)?,
        })
    }
}

fn hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// An image is either an immutable image id (`sha256:` and 64 lowercase hex digits, what a local tag resolves to at
/// start) or `<repository>@sha256:<64 hex>`: a tag alone could move.
fn pinned(name: &str, reference: &str) -> Result<String, String> {
    if reference.strip_prefix("sha256:").is_some_and(hex64) {
        return Ok(reference.to_owned());
    }
    let refuse = || format!("{name} must be <repository>@sha256:<64 lowercase hex> or a local tag");
    let (repository, digest) = reference.split_once("@sha256:").ok_or_else(refuse)?;
    let repository_ok = !repository.is_empty()
        && repository.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-/:".contains(&b));
    if !repository_ok || !hex64(digest) {
        return Err(refuse());
    }
    Ok(reference.to_owned())
}

/// A plain local tag with an explicit tag, such as `easi-run/display:dev`, which the controller resolves
/// once at start to its image id.
pub fn local_tag(reference: &str) -> bool {
    let Some((repository, tag)) = reference.rsplit_once(':') else { return false };
    let repository_ok = !repository.is_empty()
        && !repository.starts_with(['/', '.', '-'])
        && repository.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-/".contains(&b));
    let tag_ok = (1..=128).contains(&tag.len())
        && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !tag.starts_with(['.', '-']);
    repository_ok && tag_ok && !reference.contains('@')
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn images_must_be_pinned_by_digest() {
        assert!(pinned("X", &format!("example.test/a@sha256:{DIGEST}")).is_ok());
        assert!(pinned("X", "example.test/a:latest").is_err());
        assert!(pinned("X", "example.test/a@sha256:abc").is_err());
        assert!(pinned("X", &format!("example.test/a@sha256:{}", DIGEST.to_uppercase())).is_err());
        assert!(pinned("X", &format!("@sha256:{DIGEST}")).is_err());
        assert!(pinned("X", &format!("a b@sha256:{DIGEST}")).is_err());
        assert!(pinned("X", &format!("sha256:{DIGEST}")).is_ok());
        assert!(pinned("X", "sha256:abc").is_err());
        assert!(pinned("X", "easi-run/display:dev").is_err(), "a tag is resolved before, never used");
    }

    #[test]
    fn local_tags() {
        for tag in ["easi-run/display:dev", "busybox:1.37", "a/b/c:v1.2-rc_3"] {
            assert!(local_tag(tag), "{tag}");
        }
        for tag in [
            "busybox",
            "busybox:",
            ":dev",
            "Display:dev",
            "a@sha256:00:dev",
            "a:b c",
            "a:-x",
            "/a:dev",
            "localhost:5000/a",
        ] {
            assert!(!local_tag(tag), "{tag}");
        }
    }
}
