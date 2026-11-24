#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::plugin_source;
use crate::api::claude_plugin_policy::source::PluginSource;

const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

// a known key that fell through to the extra map still serializes, so only the typed source shows it
#[test]
fn plugin_source_keeps_the_known_fields_and_the_extras_apart() {
    let manifest_source = Json::Object(IndexMap::from([
        ("source".to_owned(), text("archive")),
        ("repo".to_owned(), text("org/repo")),
        ("url".to_owned(), text("https://example.com/repo.git")),
        ("path".to_owned(), text("plugins/name")),
        ("sha256".to_owned(), text(SHA256)),
        ("branch".to_owned(), text("main")),
    ]));
    let source = plugin_source(manifest_source).expect("the source object decodes");
    assert_eq!(
        source,
        PluginSource {
            source: Some("archive".to_owned()),
            repo: Some("org/repo".to_owned()),
            url: Some("https://example.com/repo.git".to_owned()),
            path: Some("plugins/name".to_owned()),
            sha256: Some(SHA256.to_owned()),
            extra: BTreeMap::from([("branch".to_owned(), "main".to_owned())]),
        }
    );
}
