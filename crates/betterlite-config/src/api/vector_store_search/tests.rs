#![cfg(test)]

use std::collections::BTreeMap;

use super::Settings;

fn environment(value: Option<&str>) -> BTreeMap<String, String> {
    value
        .map(|text| BTreeMap::from([("REQUEST_TIMEOUT".to_owned(), text.to_owned())]))
        .unwrap_or_default()
}

#[test]
fn loads_search_timeout() {
    let default = Settings::load(&environment(None)).expect("an unset timeout is the default");
    assert_eq!(
        default.request_timeout_seconds.to_bits(),
        6000.0_f64.to_bits()
    );
    let decimal = Settings::load(&environment(Some("7.5"))).expect("a decimal timeout loads");
    assert_eq!(decimal.request_timeout_seconds.to_bits(), 7.5_f64.to_bits());
    let error = Settings::load(&environment(Some("bad"))).expect_err("a malformed timeout fails");
    assert!(error.to_string().contains("REQUEST_TIMEOUT"));
}
