#![cfg(test)]

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;

use crate::api::agents::{AgentsApiError, AgentsProvider, get_agents_api_config};

const UNSUPPORTED_MESSAGE: &str = "Provider 'openai' does not have a native agents API. Use the proxy POST /v1/agents endpoint to store agents locally.";

#[test]
fn a_supported_provider_returns_its_config() {
    assert_eq!(get_agents_api_config("gemini"), Ok(AgentsProvider::Gemini));
}

#[test]
fn an_unsupported_provider_names_the_proxy_endpoint() {
    let error = get_agents_api_config("openai").expect_err("openai has no native agents API");
    assert_eq!(error.to_string(), UNSUPPORTED_MESSAGE);
    assert_eq!(
        error,
        AgentsApiError::UnsupportedProvider {
            provider: "openai".to_owned()
        }
    );
}

#[test]
fn the_unsupported_provider_error_is_a_bad_request() {
    let error = get_agents_api_config("mistral").expect_err("mistral has no native agents API");
    let class = error.class();
    let ErrorClass::BadRequest(details) = &class else {
        panic!("the unsupported provider raises a BadRequestError");
    };
    assert_eq!(
        details.message,
        "Provider 'mistral' does not have a native agents API. Use the proxy POST /v1/agents endpoint to store agents locally."
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("mistral"));
    assert_eq!(details.status.0, 400);
}

#[test]
fn the_bad_request_keeps_type_and_param_null() {
    let class = get_agents_api_config("openai")
        .expect_err("openai has no native agents API")
        .class();
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "400");
    assert_eq!(metadata.model.as_deref(), Some(""));
    assert_eq!(metadata.provider.as_deref(), Some("openai"));
}
