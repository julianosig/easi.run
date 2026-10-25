//! Where a browser says it is: its time zone and languages, which sites compare with each other and with where its
//! traffic comes from. The admin application checks its configuration with these, the controller every session it
//! starts, and the browser's launcher what it is given.

/// Languages a browser may name.
pub const MAX_LANGUAGES: usize = 8;

/// A time zone of the IANA database (as `America/Sao_Paulo`), which Chrome's own copy of it knows too.
pub fn valid_timezone(name: &str) -> bool {
    // The database also matches names case-insensitively; only its exact spelling is a name Chrome reports back.
    jiff_tzdb::get(name).is_some_and(|(canonical, _)| canonical == name)
}

/// A BCP 47 language tag of the forms browsers send: `pt`, `pt-BR`, `zh-Hant-TW`, `es-419`.
fn valid_language(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let primary = parts.next().unwrap_or_default();
    (2..=3).contains(&primary.len())
        && primary.bytes().all(|b| b.is_ascii_alphabetic())
        && tag.split('-').count() <= 3
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// One to `MAX_LANGUAGES` tags, preferred first.
pub fn valid_languages(tags: &[String]) -> bool {
    (1..=MAX_LANGUAGES).contains(&tags.len()) && tags.iter().all(|t| valid_language(t))
}

/// The `LANGUAGE` list Chrome on Linux picks its interface and default page locale from (`pt_BR:pt:en_US:en`).
pub fn gettext_list(tags: &[String]) -> String {
    tags.iter().map(|t| t.replace('-', "_")).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_zones_are_the_databases_names_only() {
        for good in ["UTC", "America/Sao_Paulo", "America/Argentina/Buenos_Aires", "Europe/London", "Etc/GMT+3"] {
            assert!(valid_timezone(good), "{good}");
        }
        for bad in ["", "../etc/passwd", "America/DefinitelyNotAZone", "america/sao_paulo", "/UTC", "UTC\n"] {
            assert!(!valid_timezone(bad), "{bad:?}");
        }
    }

    #[test]
    fn languages_are_tags_only() {
        for good in ["pt", "pt-BR", "en-US", "zh-Hant-TW", "es-419"] {
            assert!(valid_language(good), "{good}");
        }
        for bad in ["", "p", "portuguese", "pt_BR", "pt-", "en-US;q=0.9", "a-b-c-d", "pt-BR,en", "pt-BR:en"] {
            assert!(!valid_language(bad), "{bad:?}");
        }
        assert!(!valid_languages(&[]));
        assert!(!valid_languages(&vec!["en".to_owned(); MAX_LANGUAGES + 1]));
        let tags = ["pt-BR", "pt", "en-US", "en"].map(str::to_owned);
        assert!(valid_languages(&tags));
        assert_eq!(gettext_list(&tags), "pt_BR:pt:en_US:en");
    }
}
