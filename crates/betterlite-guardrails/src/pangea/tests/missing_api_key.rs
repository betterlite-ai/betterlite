#![cfg(test)]

use std::collections::BTreeMap;

use crate::pangea::config::PangeaGuardrailParams;
use crate::pangea::error::PangeaGuardrailError;
use crate::pangea::hooks::PangeaGuardrail;
use crate::pangea::tests::support::GUARDRAIL_NAME;

const MISSING_KEY_MESSAGE: &str = "Pangea API Key not found. Set PANGEA_API_KEY environment variable or pass it in litellm_params.";

#[test]
fn missing_api_key_raises_the_pangea_missing_secrets_error() {
    let error = PangeaGuardrail::new(
        Some(GUARDRAIL_NAME.to_owned()),
        PangeaGuardrailParams::default(),
        &BTreeMap::new(),
        reqwest::Client::default(),
    )
    .expect_err("a guardrail without a key does not initialize");
    assert!(matches!(error, PangeaGuardrailError::MissingSecrets));
    assert_eq!(error.to_string(), MISSING_KEY_MESSAGE);
}
