#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::loader::EnvironmentLookup;

use super::{Borrowed, loads_read_timeout};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn seconds(value: f64) -> Duration {
    Duration::try_from_secs_f64(value).expect("the timeout is a whole number of seconds")
}

// an environment whose only extra is the home lookup the loader forwards
struct Home(PathBuf);

impl EnvironmentLookup for Home {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }

    fn system_home(&self) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

#[test]
fn forwards_the_system_home() {
    let home = Home(PathBuf::from("/opt/litellm"));
    assert_eq!(
        Borrowed(&home).system_home(),
        Some(PathBuf::from("/opt/litellm"))
    );
}

#[test]
fn loads_read_timeout() {
    let unset = environment(&[]);
    let default_seconds = loads_read_timeout(&unset).expect("the default loads");
    assert_eq!(seconds(default_seconds), Duration::from_mins(100));
    let configured = environment(&[("REQUEST_TIMEOUT", "12")]);
    let environment_seconds = loads_read_timeout(&configured).expect("the value loads");
    assert_eq!(seconds(environment_seconds), Duration::from_secs(12));
    let invalid = environment(&[("REQUEST_TIMEOUT", "bad")]);
    let error = loads_read_timeout(&invalid).expect_err("the value is not a number");
    assert!(error.to_string().contains("REQUEST_TIMEOUT"));
    assert!(error.to_string().contains("must be a number"));
}
