#![cfg(test)]

use std::collections::BTreeSet;

use crate::search::BaseSearchConfig;

#[test]
fn base_defaults_and_noop_hooks() {
    let config = BaseSearchConfig;
    assert_eq!(
        BaseSearchConfig::ui_friendly_name(),
        "Unknown Search Provider"
    );
    assert!(!config.supports_rich_search_input());
    assert_eq!(config.get_http_method(), "POST");
    let headers = BTreeSet::from([(String::from("x-test"), String::from("yes"))]);
    assert_eq!(
        config.validate_environment(headers.clone(), None, None),
        headers
    );
    assert_eq!(config.sign_request(headers.clone()), (headers, None));
}
