use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

pub(super) fn object(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("a JSON fixture")
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
