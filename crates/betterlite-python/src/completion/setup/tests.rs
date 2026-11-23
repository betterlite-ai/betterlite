#![cfg(test)]

use super::global_credentials;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_sdk::completion_request::args::{CompletionFlags, CompletionSettings};
use secrecy::{ExposeSecret, SecretString};
use std::collections::BTreeMap;
#[test]
fn credentials_read_the_settings_key_and_base() {
    let settings = CompletionSettings {
        drop_params: None,
        model_fallbacks: None,
        model_alias_map: BTreeMap::new(),
        num_retries: None,
        request_timeout: None,
        api_key: Some(SecretString::from(String::from("sk-module"))),
        api_base: Some(String::from("http://module")),
        route_all_to_responses: false,
        proxy_headers: AdditionalHeaders::new(),
        optional: OptionalParams::default(),
        flags: CompletionFlags::default(),
        medium_thinking_budget: 0,
    };
    let globals = global_credentials(&settings);
    assert_eq!(
        globals.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-module")
    );
    assert_eq!(globals.api_base.as_deref(), Some("http://module"));
}
