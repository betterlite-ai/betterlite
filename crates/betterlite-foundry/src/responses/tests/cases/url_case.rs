use crate::credentials::model_info::ModelInfoAccessors;
use crate::responses::path::PathError;
use crate::responses::url::{ResponsesUrlError, complete_url};

const MISSING: &str = "api_base is required for the Azure AI Foundry Responses API. Set the api_base parameter or the AZURE_AI_API_BASE environment variable.";

pub(super) fn foundry_responses_url() {
    foundry_base();
    project_path();
    discarded_path();
    query_dropped_fragment_kept();
    resolved_base();
    missing_base();
    invalid_base();
}

fn foundry_base() {
    let sources = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(sources, Some("https://res.services.ai.azure.com")).unwrap(),
        "https://res.services.ai.azure.com/openai/v1/responses"
    );
    assert_eq!(
        complete_url(sources, Some("https://res.services.ai.azure.com/")).unwrap(),
        "https://res.services.ai.azure.com/openai/v1/responses"
    );
}

fn project_path() {
    let sources = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(
            sources,
            Some("https://res.services.ai.azure.com/api/projects/proj")
        )
        .unwrap(),
        "https://res.services.ai.azure.com/api/projects/proj/openai/v1/responses"
    );
    assert_eq!(
        complete_url(
            sources,
            Some("https://res.services.ai.azure.com/api/projects")
        )
        .unwrap(),
        "https://res.services.ai.azure.com/api/projects/openai/v1/responses"
    );
    assert_eq!(
        complete_url(
            sources,
            Some("https://res.services.ai.azure.com//api//projects//proj//")
        )
        .unwrap(),
        "https://res.services.ai.azure.com/api/projects/proj/openai/v1/responses"
    );
    assert_eq!(
        complete_url(
            sources,
            Some("https://res.services.ai.azure.com/api/projects/p%2Fx")
        )
        .unwrap(),
        "https://res.services.ai.azure.com/api/projects/p/openai/v1/responses"
    );
}

fn discarded_path() {
    let sources = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(sources, Some("https://res.services.ai.azure.com/foo/bar/")).unwrap(),
        "https://res.services.ai.azure.com/openai/v1/responses"
    );
    assert_eq!(
        complete_url(
            sources,
            Some("https://res.services.ai.azure.com/api/project/proj")
        )
        .unwrap(),
        "https://res.services.ai.azure.com/openai/v1/responses"
    );
}

fn query_dropped_fragment_kept() {
    let sources = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(
            sources,
            Some(
                "https://res.services.ai.azure.com/api/projects/proj/models/x?api-version=2024#frag"
            )
        )
        .unwrap(),
        "https://res.services.ai.azure.com/api/projects/proj/openai/v1/responses#frag"
    );
    assert_eq!(
        complete_url(
            sources,
            Some("https://res.services.ai.azure.com/openai/v1/responses?api-version=1")
        )
        .unwrap(),
        "https://res.services.ai.azure.com/openai/v1/responses"
    );
}

fn resolved_base() {
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
    let global = ModelInfoAccessors {
        global_api_base: Some("https://global.services.ai.azure.com"),
        global_api_key: None,
        environment_api_base: Some("https://env.services.ai.azure.com"),
        environment_api_key: None,
    };
    assert_eq!(
        complete_url(global, None).unwrap(),
        "https://global.services.ai.azure.com/openai/v1/responses"
    );
    assert_eq!(
        complete_url(global, Some("https://explicit.services.ai.azure.com")).unwrap(),
        "https://explicit.services.ai.azure.com/openai/v1/responses"
    );
}

fn missing_base() {
    let sources = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    let error = complete_url(sources, None).unwrap_err();
    assert_eq!(error.to_string(), MISSING);
    assert!(matches!(&error, ResponsesUrlError::MissingApiBase));
}

fn invalid_base() {
    let sources = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    let error = complete_url(sources, Some("https://host:port")).unwrap_err();
    assert!(matches!(error, ResponsesUrlError::InvalidUrl(_)));
    // httpx refuses to copy a path holding a reserved character, so the rewrite fails instead of guessing
    let unrewritable = complete_url(
        sources,
        Some("https://res.services.ai.azure.com/api/projects/pro%3Fj"),
    )
    .unwrap_err();
    assert!(matches!(
        unrewritable,
        ResponsesUrlError::Path(PathError::Unrewritable)
    ));
}
