#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

#[test]
fn loads_token_rate_limit_type_from_general_settings() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  token_rate_limit_type: input\n")
    .expect("config should load");
    assert_eq!(
        config.sections.rate_limit_usage.token_rate_limit_type,
        "input"
    );
}
