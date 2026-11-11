//! Presentation metadata taken from a verified manifest.
//!
//! The values come from a package whose signature and checksums were already
//! verified, but they are still display text: they are bounded, stripped of
//! control characters and never used for authorization decisions.

use std::collections::BTreeMap;

pub const MAX_NAME_CHARS: usize = 120;
pub const MAX_SUMMARY_CHARS: usize = 500;
pub const MAX_CATEGORIES: usize = 8;
pub const MAX_CATEGORY_CHARS: usize = 32;
pub const MAX_AUTHORS: usize = 8;
pub const MAX_AUTHOR_CHARS: usize = 80;
pub const MAX_DEPENDENCIES: usize = 32;
pub const MAX_DEPENDENCY_CHARS: usize = 128;
pub const MAX_ENGINE_CHARS: usize = 64;
pub const MAX_HOMEPAGE_CHARS: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presentation {
    pub name: String,
    pub summary: Option<String>,
    pub categories: Vec<String>,
    pub authors: Vec<String>,
    pub engine: Option<String>,
    pub homepage: Option<String>,
    pub dependencies: BTreeMap<String, String>,
}

/// Raw manifest fields as read from the verified package.
pub struct RawManifest<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub categories: &'a [String],
    pub authors: Vec<&'a str>,
    pub engine: Option<&'a str>,
    pub homepage: Option<&'a str>,
    pub dependencies: &'a BTreeMap<String, String>,
}

pub fn sanitize(raw: &RawManifest<'_>) -> Presentation {
    let name = text(raw.name, MAX_NAME_CHARS).unwrap_or_default();
    Presentation {
        name,
        summary: raw.description.and_then(|s| text(s, MAX_SUMMARY_CHARS)),
        categories: raw
            .categories
            .iter()
            .filter_map(|c| category(c))
            .take(MAX_CATEGORIES)
            .collect(),
        authors: raw
            .authors
            .iter()
            .filter_map(|a| text(a, MAX_AUTHOR_CHARS))
            .take(MAX_AUTHORS)
            .collect(),
        engine: raw.engine.and_then(|e| text(e, MAX_ENGINE_CHARS)),
        homepage: raw.homepage.and_then(https_url),
        dependencies: raw
            .dependencies
            .iter()
            .filter_map(|(k, v)| {
                Some((
                    text(k, MAX_DEPENDENCY_CHARS)?,
                    text(v, MAX_DEPENDENCY_CHARS)?,
                ))
            })
            .take(MAX_DEPENDENCIES)
            .collect(),
    }
}

/// Trims, rejects control characters and bounds the length in characters.
fn text(value: &str, max_chars: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

/// Categories are machine-facing filter keys: lowercase ASCII words only.
fn category(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let valid = !value.is_empty()
        && value.len() <= MAX_CATEGORY_CHARS
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    valid.then_some(value)
}

/// Only HTTPS links are displayed; anything else is dropped rather than rewritten.
fn https_url(value: &str) -> Option<String> {
    let value = text(value, MAX_HOMEPAGE_CHARS)?;
    let rest = value.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let valid = !host.is_empty() && !value.chars().any(char::is_whitespace) && !host.contains('@');
    valid.then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw<'a>(deps: &'a BTreeMap<String, String>, cats: &'a [String]) -> RawManifest<'a> {
        RawManifest {
            name: "  Gift Alerts  ",
            description: Some("Plays a sound\u{0007} on gifts"),
            categories: cats,
            authors: vec!["Ana", "", " Bo "],
            engine: Some("1.4"),
            homepage: Some("http://example.test"),
            dependencies: deps,
        }
    }

    #[test]
    fn trims_strips_controls_and_drops_empty_values() {
        let deps = BTreeMap::new();
        let cats: Vec<String> = vec![];
        let p = sanitize(&raw(&deps, &cats));
        assert_eq!(p.name, "Gift Alerts");
        assert_eq!(
            p.summary, None,
            "control characters make the whole summary unusable"
        );
        assert_eq!(p.authors, vec!["Ana", "Bo"]);
        assert_eq!(p.homepage, None, "plain http is never displayed");
    }

    #[test]
    fn categories_are_lowercase_ascii_and_bounded() {
        let deps = BTreeMap::new();
        let cats = vec![
            "Audio".to_string(),
            "bad category".to_string(),
            "moderation".to_string(),
            "x".repeat(40),
        ];
        let p = sanitize(&RawManifest {
            categories: &cats,
            ..raw(&deps, &cats)
        });
        assert_eq!(p.categories, vec!["audio", "moderation"]);
    }

    #[test]
    fn long_text_is_truncated_on_character_boundaries() {
        let deps = BTreeMap::new();
        let cats: Vec<String> = vec![];
        let name = "é".repeat(200);
        let p = sanitize(&RawManifest {
            name: &name,
            ..raw(&deps, &cats)
        });
        assert_eq!(p.name.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn https_homepage_and_bounded_dependencies_are_kept() {
        let mut deps = BTreeMap::new();
        for i in 0..40 {
            deps.insert(format!("dep{i:02}"), "^1".to_string());
        }
        let cats: Vec<String> = vec![];
        let p = sanitize(&RawManifest {
            homepage: Some("https://example.test/plugin"),
            ..raw(&deps, &cats)
        });
        assert_eq!(p.homepage.as_deref(), Some("https://example.test/plugin"));
        assert_eq!(p.dependencies.len(), MAX_DEPENDENCIES);
    }
}
