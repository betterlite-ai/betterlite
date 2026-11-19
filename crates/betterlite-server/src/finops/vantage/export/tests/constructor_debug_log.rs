#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;
use tracing::Level;

use crate::finops::vantage::support::{self, Recorder};

#[rstest]
#[case::long_token(
    Some("abcdefgh"),
    "VantageLogger initialized (integration_token=abcd***)"
)]
#[case::four_characters(Some("abcd"), "VantageLogger initialized (integration_token=***)")]
#[case::short_token(Some("abc"), "VantageLogger initialized (integration_token=***)")]
#[case::absent(None, "VantageLogger initialized (integration_token=***)")]
fn the_initialization_log_masks_the_integration_token(
    #[case] token: Option<&str>,
    #[case] expected: &str,
) {
    let (logs, _guard) = support::capture_logs();
    let mut params = support::params();
    params.integration_token = token.map(SecretString::from);
    let _logger = support::build(&BTreeMap::new(), params, &Recorder::new(false), None);
    assert!(logs.has_at(Level::DEBUG, expected));
}
