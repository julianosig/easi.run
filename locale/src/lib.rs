//! Where a browser says it is: its time zone and languages, which sites compare with each other and with where its
//! traffic comes from. The admin application checks its configuration with these, the controller every session it
//! starts, and the browser's launcher what it is given.

mod countries;

pub use countries::{COUNTRIES, Country};

/// Languages a browser may name.
pub const MAX_LANGUAGES: usize = 8;

/// The country with this ISO 3166-1 alpha-2 code, if listed.
pub fn country(code: &str) -> Option<&'static Country> {
    COUNTRIES.iter().find(|c| c.code == code)
}

/// The languages every suggestion ends with.
const FALLBACK_LANGUAGES: &[&str] = &["en-US", "en"];

/// Where browsers leaving from the country `code` should say they are: its time zone, and its languages followed by
/// `en-US` and `en` (at most [`MAX_LANGUAGES`] in all); `UTC` and `en-US, en` for a country not listed.
pub fn suggested(code: &str) -> (String, Vec<String>) {
    let Some(country) = country(code) else {
        return ("UTC".to_owned(), FALLBACK_LANGUAGES.iter().map(|&tag| tag.to_owned()).collect());
    };
    let mut languages: Vec<String> = Vec::new();
    for tag in country.languages.iter().chain(FALLBACK_LANGUAGES) {
        if !languages.iter().any(|l| l == tag) && languages.len() < MAX_LANGUAGES {
            languages.push((*tag).to_owned());
        }
    }
    (country.timezone.to_owned(), languages)
}

/// The time zones offered first when a person picks one: the most used.
pub const COMMON_TIMEZONES: &[&str] = &[
    "UTC",
    "America/New_York",
    "America/Chicago",
    "America/Denver",
    "America/Los_Angeles",
    "America/Phoenix",
    "America/Anchorage",
    "Pacific/Honolulu",
    "America/Toronto",
    "America/Vancouver",
    "America/Mexico_City",
    "America/Bogota",
    "America/Lima",
    "America/Santiago",
    "America/Argentina/Buenos_Aires",
    "America/Sao_Paulo",
    "Europe/London",
    "Europe/Dublin",
    "Europe/Lisbon",
    "Europe/Madrid",
    "Europe/Paris",
    "Europe/Amsterdam",
    "Europe/Brussels",
    "Europe/Berlin",
    "Europe/Zurich",
    "Europe/Rome",
    "Europe/Stockholm",
    "Europe/Warsaw",
    "Europe/Athens",
    "Europe/Istanbul",
    "Europe/Kyiv",
    "Europe/Moscow",
    "Africa/Cairo",
    "Africa/Lagos",
    "Africa/Johannesburg",
    "Asia/Dubai",
    "Asia/Kolkata",
    "Asia/Bangkok",
    "Asia/Jakarta",
    "Asia/Singapore",
    "Asia/Hong_Kong",
    "Asia/Shanghai",
    "Asia/Taipei",
    "Asia/Seoul",
    "Asia/Tokyo",
    "Australia/Sydney",
    "Pacific/Auckland",
];

/// The languages offered first when a person picks them, the most used, each tag with its English name.
pub const COMMON_LANGUAGES: &[(&str, &str)] = &[
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("en", "English"),
    ("es-ES", "Spanish (Spain)"),
    ("es-MX", "Spanish (Mexico)"),
    ("es-419", "Spanish (Latin America)"),
    ("es", "Spanish"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("pt-PT", "Portuguese (Portugal)"),
    ("pt", "Portuguese"),
    ("fr-FR", "French (France)"),
    ("fr-CA", "French (Canada)"),
    ("fr", "French"),
    ("de-DE", "German (Germany)"),
    ("de", "German"),
    ("it-IT", "Italian"),
    ("nl-NL", "Dutch"),
    ("pl-PL", "Polish"),
    ("ru-RU", "Russian"),
    ("uk-UA", "Ukrainian"),
    ("tr-TR", "Turkish"),
    ("ar", "Arabic"),
    ("he-IL", "Hebrew"),
    ("hi-IN", "Hindi"),
    ("id-ID", "Indonesian"),
    ("th-TH", "Thai"),
    ("vi-VN", "Vietnamese"),
    ("zh-CN", "Chinese (Simplified)"),
    ("zh-TW", "Chinese (Traditional)"),
    ("ja-JP", "Japanese"),
    ("ko-KR", "Korean"),
    ("sv-SE", "Swedish"),
];

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
    fn every_country_and_offered_choice_is_valid() {
        assert!(COUNTRIES.len() > 230);
        for (i, c) in COUNTRIES.iter().enumerate() {
            let code_ok = c.code.len() == 2 && c.code.bytes().all(|b| b.is_ascii_uppercase());
            assert!(code_ok && !c.name.is_empty(), "{}", c.code);
            assert!(i == 0 || COUNTRIES[i - 1].code < c.code, "{} is out of order or repeated", c.code);
            assert!(valid_timezone(c.timezone), "{}: {}", c.code, c.timezone);
            let (timezone, languages) = suggested(c.code);
            assert_eq!(timezone, c.timezone);
            assert!(valid_languages(&languages), "{}: {languages:?}", c.code);
            assert_eq!(&languages[languages.len() - 2..], ["en-US", "en"], "{}", c.code);
        }
        for zone in COMMON_TIMEZONES {
            assert!(valid_timezone(zone), "{zone}");
        }
        for (tag, name) in COMMON_LANGUAGES {
            assert!(valid_language(tag) && !name.is_empty(), "{tag}");
        }
    }

    #[test]
    fn a_country_suggests_its_place_and_an_unknown_one_utc() {
        assert_eq!(
            suggested("BR"),
            ("America/Sao_Paulo".to_owned(), ["pt-BR", "pt", "en-US", "en"].map(str::to_owned).to_vec())
        );
        assert_eq!(suggested("US"), ("America/New_York".to_owned(), ["en-US", "en"].map(str::to_owned).to_vec()));
        assert_eq!(suggested("T1"), ("UTC".to_owned(), ["en-US", "en"].map(str::to_owned).to_vec()));
        assert_eq!(country("DE").map(|c| c.name), Some("Germany"));
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
