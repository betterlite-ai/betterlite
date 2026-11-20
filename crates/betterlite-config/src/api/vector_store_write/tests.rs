#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use super::loads_request_timeout;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn seconds(value: f64) -> Duration {
    Duration::try_from_secs_f64(value).expect("the timeout is a whole number of seconds")
}

#[test]
fn loads_request_timeout() {
    let unset = environment(&[]);
    let default_seconds = loads_request_timeout(&unset).expect("the default loads");
    assert_eq!(seconds(default_seconds), Duration::from_secs(6000));
    let configured = environment(&[("REQUEST_TIMEOUT", "12")]);
    let environment_seconds = loads_request_timeout(&configured).expect("the value loads");
    assert_eq!(seconds(environment_seconds), Duration::from_secs(12));
    let invalid = environment(&[("REQUEST_TIMEOUT", "bad")]);
    let error = loads_request_timeout(&invalid).expect_err("the value is not a number");
    assert!(error.to_string().contains("REQUEST_TIMEOUT"));
    assert!(error.to_string().contains("must be a number"));
}
