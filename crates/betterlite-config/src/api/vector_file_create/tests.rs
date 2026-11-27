#![cfg(test)]

use std::collections::BTreeMap;

use crate::api::vector_store_write::loads_request_timeout as load_request_timeout;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn loads_request_timeout() {
    let unset = environment(&[]);
    let default_seconds = load_request_timeout(&unset).expect("the default loads");
    assert_eq!(default_seconds, 6000.0_f64);
    let configured = environment(&[("REQUEST_TIMEOUT", "12")]);
    let environment_seconds = load_request_timeout(&configured).expect("the value loads");
    assert_eq!(environment_seconds, 12.0_f64);
    let invalid = environment(&[("REQUEST_TIMEOUT", "bad")]);
    let error = load_request_timeout(&invalid).expect_err("the value is not a number");
    assert!(error.to_string().contains("REQUEST_TIMEOUT"));
    assert!(error.to_string().contains("must be a number"));
}
