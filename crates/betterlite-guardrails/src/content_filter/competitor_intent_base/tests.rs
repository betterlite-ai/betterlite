#![cfg(test)]

use super::Meaning;
use super::configuration::Settings;

pub(super) fn config() -> Settings {
    serde_json::from_str(
        r#"{
        "brand_self": ["Our Brand"],
        "competitors": ["Acme"],
        "competitor_aliases": {"acme": ["AC"]},
        "domain_words": ["airline"],
        "locations": ["acme"]
    }"#,
    )
    .unwrap()
}

pub(super) fn competitor(_text: &str, _token: &str) -> (Meaning, f64) {
    (Meaning::Competitor, 0.9)
}
