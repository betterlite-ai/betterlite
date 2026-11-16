use std::hint::black_box;

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra};
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::decisions::transformation::wire::ProviderRequestWire;

use crate::responses::decisions::AzureAiDecisionsConfig;

const ROUTE: &str = "/providers/microsoft/v1/systemone";
const MISSING: &str = "Missing AZURE_AI_API_BASE - set AZURE_AI_API_BASE or pass api_base, e.g. https://<resource>.services.ai.azure.com";
const SECRETS: [(&str, &str); 2] = [
    ("AZURE_AI_API_BASE", "https://env.services.ai.azure.com"),
    ("AZURE_AI_API_KEY", "env-key"),
];

pub(in crate::responses::tests) fn decisions_route() {
    metadata();
    rendered_routes();
    retained_paths();
    credential_resolution();
    missing_message();
    unparsable_base();
    systemone_wire();
}

fn metadata() {
    assert_eq!(AzureAiDecisionsConfig::PATH, ROUTE);
    assert_eq!(
        AzureAiDecisionsConfig::API_KEY_ENV,
        ["AZURE_AI_API_KEY"].as_slice()
    );
    assert_eq!(
        AzureAiDecisionsConfig::API_BASE_ENV,
        ["AZURE_AI_API_BASE"].as_slice()
    );
    assert!(black_box(AzureAiDecisionsConfig::API_KEY_REQUIRED));
}

fn url(api_base: &str) -> String {
    AzureAiDecisionsConfig.get_complete_url(api_base, "Microsoft-Decision-1")
}

fn rendered_routes() {
    root_routes();
    nested_routes();
}

fn root_routes() {
    assert_eq!(
        url("https://res.services.ai.azure.com"),
        format!("https://res.services.ai.azure.com{ROUTE}")
    );
    assert_eq!(
        url("https://res.services.ai.azure.com/"),
        format!("https://res.services.ai.azure.com{ROUTE}")
    );
    for suffix in ["/openai/v1", "/openai", "/models", "/v1"] {
        assert_eq!(
            url(&format!("https://res.services.ai.azure.com{suffix}")),
            format!("https://res.services.ai.azure.com{ROUTE}"),
            "{suffix}"
        );
    }
    assert_eq!(
        url("https://res.services.ai.azure.com/api/projects/proj"),
        format!("https://res.services.ai.azure.com{ROUTE}")
    );
    for suffix in ["/openai/v1", "/openai", "/models"] {
        assert_eq!(
            url(&format!(
                "https://res.services.ai.azure.com/api/projects/proj{suffix}"
            )),
            format!("https://res.services.ai.azure.com{ROUTE}"),
            "{suffix}"
        );
    }
}

fn nested_routes() {
    assert_eq!(
        url("https://res.services.ai.azure.com/api/projects/proj/models/v1"),
        format!("https://res.services.ai.azure.com/api/projects/proj/models{ROUTE}")
    );
    assert_eq!(
        url("https://res.services.ai.azure.com/%6Fpenai%2Fv1"),
        format!("https://res.services.ai.azure.com{ROUTE}")
    );
    assert_eq!(
        url("https://res.services.ai.azure.com/models?api-version=1#frag"),
        format!("https://res.services.ai.azure.com{ROUTE}")
    );
}

fn retained_paths() {
    assert_eq!(
        url("https://res.services.ai.azure.com/foo/bar"),
        format!("https://res.services.ai.azure.com/foo/bar{ROUTE}")
    );
    assert_eq!(
        url("https://res.services.ai.azure.com/openai/v1/responses"),
        format!("https://res.services.ai.azure.com/openai/v1/responses{ROUTE}")
    );
    assert_eq!(
        url("https://res.services.ai.azure.com/api/projects/proj/openai/v1/x"),
        format!("https://res.services.ai.azure.com/api/projects/proj/openai/v1/x{ROUTE}")
    );
}

fn fixture_secret(name: &str) -> String {
    SECRETS
        .iter()
        .find(|(key, _value)| *key == name)
        .map(|(_key, value)| (*value).to_owned())
        .unwrap_or_default()
}

const fn no_secret(_name: &str) -> String {
    String::new()
}

fn credential_resolution() {
    let config = AzureAiDecisionsConfig;
    assert_eq!(
        config.resolve_api_base(None, fixture_secret),
        Some(String::from("https://env.services.ai.azure.com"))
    );
    assert_eq!(
        config.resolve_api_base(
            Some("https://explicit.services.ai.azure.com"),
            fixture_secret
        ),
        Some(String::from("https://explicit.services.ai.azure.com"))
    );
    assert_eq!(
        config.resolve_api_key(None, fixture_secret),
        Some(String::from("env-key"))
    );
    assert_eq!(
        config.resolve_api_key(Some("explicit-key"), fixture_secret),
        Some(String::from("explicit-key"))
    );
    assert_eq!(config.resolve_api_base(None, no_secret), None);
    assert_eq!(config.resolve_api_key(None, no_secret), None);
}

fn missing_message() {
    assert_eq!(
        AzureAiDecisionsConfig.missing_api_base_message("azure_ai"),
        MISSING
    );
}

fn unparsable_base() {
    // the provider-config trait cannot fail, so a base the transport must reject comes back untouched
    assert_eq!(url("https://host:port"), "https://host:port");
    assert_eq!(
        url("https://res.services.ai.azure.com/models%3Fv"),
        "https://res.services.ai.azure.com/models%3Fv"
    );
}

fn systemone_wire() {
    let request = ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text("the sky is blue".to_owned())),
        questions: vec![ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
            name: Some("blue".to_owned()),
            instructions: Some(DecisionsJson::Text("Is the sky blue?".to_owned())),
            criteria: None,
            extra: Extra::default(),
        })],
        safety_identifier: None,
    };
    let wire = AzureAiDecisionsConfig
        .transform_decisions_request("Microsoft-Decision-1", &request, "azure_ai")
        .unwrap();
    let ProviderRequestWire::SystemOne(systemone) = wire else {
        panic!("the Foundry route keeps the SystemOne wire")
    };
    assert_eq!(systemone.model, "Microsoft-Decision-1");
    assert_eq!(
        systemone.state,
        DecisionsJson::Text("the sky is blue".to_owned())
    );
    assert_eq!(systemone.questions.len(), 1);
}
