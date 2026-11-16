use crate::credentials::model_info::ModelInfoAccessors;
use crate::responses::tests::support::no_accessors;
use crate::responses::url::{ResponsesUrlError, complete_url};

const MISSING: &str = "api_base is required for the Azure AI Foundry Responses API. Set the api_base parameter or the AZURE_AI_API_BASE environment variable.";

pub(in crate::responses::tests) fn missing_base() {
    absent_everywhere();
    resolved_bases();
}

fn absent_everywhere() {
    let error = complete_url(no_accessors(), None).unwrap_err();
    assert_eq!(error.to_string(), MISSING);
    assert!(matches!(&error, ResponsesUrlError::MissingApiBase));
}

// the three layers pick in the same order LiteLLM's get_api_base does, and a configured base always answers
fn resolved_bases() {
    let rows = [
        (
            None,
            Some("https://env.services.ai.azure.com"),
            None,
            "https://env.services.ai.azure.com/openai/v1/responses",
        ),
        (
            Some("https://global.services.ai.azure.com"),
            Some("https://env.services.ai.azure.com"),
            None,
            "https://global.services.ai.azure.com/openai/v1/responses",
        ),
        (
            Some("https://global.services.ai.azure.com"),
            None,
            Some("https://explicit.services.ai.azure.com"),
            "https://explicit.services.ai.azure.com/openai/v1/responses",
        ),
    ];
    for (global, environment, explicit, expected) in rows {
        let accessors = ModelInfoAccessors {
            global_api_base: global,
            environment_api_base: environment,
            ..no_accessors()
        };
        assert_eq!(complete_url(accessors, explicit).unwrap(), expected);
    }
}
