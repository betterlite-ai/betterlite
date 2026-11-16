use crate::responses::tests::support::accessors;
use crate::responses::url::{ResponsesUrlError, complete_url};

const MISSING: &str = "api_base is required for the Azure AI Foundry Responses API. Set the api_base parameter or the AZURE_AI_API_BASE environment variable.";

pub(super) fn missing_base() {
    absent_everywhere();
    environment_only();
    global_beats_environment();
    explicit_beats_global();
}

fn absent_everywhere() {
    let error = complete_url(accessors(None, None, None, None), None).unwrap_err();
    assert_eq!(error.to_string(), MISSING);
    assert!(matches!(&error, ResponsesUrlError::MissingApiBase));
}

fn environment_only() {
    let environment = accessors(None, None, Some("https://env.services.ai.azure.com"), None);
    assert_eq!(
        complete_url(environment, None).unwrap(),
        "https://env.services.ai.azure.com/openai/v1/responses"
    );
}

fn global_beats_environment() {
    let layered = accessors(
        Some("https://global.services.ai.azure.com"),
        None,
        Some("https://env.services.ai.azure.com"),
        None,
    );
    assert_eq!(
        complete_url(layered, None).unwrap(),
        "https://global.services.ai.azure.com/openai/v1/responses"
    );
}

fn explicit_beats_global() {
    let layered = accessors(
        Some("https://global.services.ai.azure.com"),
        None,
        None,
        None,
    );
    assert_eq!(
        complete_url(layered, Some("https://explicit.services.ai.azure.com")).unwrap(),
        "https://explicit.services.ai.azure.com/openai/v1/responses"
    );
}
