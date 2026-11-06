#![cfg(test)]

use std::collections::BTreeMap;

use crate::pangea::tests::support::params;
use crate::pangea::{PangeaGuardrail, PangeaGuardrailError};

const MISSING_KEY_MESSAGE: &str = "Pangea API Key not found. Set PANGEA_API_KEY environment variable or pass it in litellm_params.";

#[test]
fn missing_api_key_raises_the_pangea_missing_secrets_error() {
    let lookup = BTreeMap::new();
    let error = PangeaGuardrail::new(params(), &lookup, reqwest::Client::default())
        .expect_err("a guardrail without a key does not initialize");
    assert!(matches!(error, PangeaGuardrailError::MissingSecrets));
    assert_eq!(error.to_string(), MISSING_KEY_MESSAGE);
}
