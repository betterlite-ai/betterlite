#![cfg(test)]

use std::collections::BTreeMap;

use super::{SettingsError, loads_read_timeout};

fn environment(value: Option<&str>) -> BTreeMap<String, String> {
    value
        .map(|text| BTreeMap::from([(String::from("REQUEST_TIMEOUT"), text.to_owned())]))
        .unwrap_or_default()
}

#[test]
fn loads_read_timeout() {
    let unset = loads_read_timeout(&environment(None)).expect("an unset timeout is the default");
    assert_eq!(unset.to_bits(), 6000.0_f64.to_bits());
    let decimal = loads_read_timeout(&environment(Some("4.5"))).expect("a decimal timeout loads");
    assert_eq!(decimal.to_bits(), 4.5_f64.to_bits());
    let error =
        loads_read_timeout(&environment(Some("bad"))).expect_err("a malformed timeout fails");
    assert!(matches!(error, SettingsError::Timeout(_)));
    assert!(error.to_string().contains("REQUEST_TIMEOUT"));
}
