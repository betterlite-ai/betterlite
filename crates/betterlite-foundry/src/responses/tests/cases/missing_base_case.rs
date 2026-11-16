use crate::credentials::model_info::ModelInfoAccessors;
use crate::responses::url::{ResponsesUrlError, complete_url};

const MISSING: &str = "api_base is required for the Azure AI Foundry Responses API. Set the api_base parameter or the AZURE_AI_API_BASE environment variable.";

pub(super) fn missing_base() {
    absent_everywhere();
    environment_only();
    global_beats_environment();
    explicit_beats_global();
}

fn absent_everywhere() {
    let error = complete_url(
        ModelInfoAccessors {
            global_api_base: None,
            global_api_key: None,
            environment_api_base: None,
            environment_api_key: None,
        },
        None,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), MISSING);
    assert!(matches!(&error, ResponsesUrlError::MissingApiBase));
}

fn environment_only() {
    let environment = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: Some("https://env.services.ai.azure.com"),
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(environment, None).unwrap(),
        "https://env.services.ai.azure.com/openai/v1/responses"
    );
}

fn global_beats_environment() {
    let layered = ModelInfoAccessors {
        global_api_base: Some("https://global.services.ai.azure.com"),
        global_api_key: None,
        environment_api_base: Some("https://env.services.ai.azure.com"),
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(layered, None).unwrap(),
        "https://global.services.ai.azure.com/openai/v1/responses"
    );
}

fn explicit_beats_global() {
    let layered = ModelInfoAccessors {
        global_api_base: Some("https://global.services.ai.azure.com"),
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(layered, Some("https://explicit.services.ai.azure.com")).unwrap(),
        "https://explicit.services.ai.azure.com/openai/v1/responses"
    );
}
