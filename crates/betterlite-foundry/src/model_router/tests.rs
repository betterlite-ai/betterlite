#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::foundry_credentials::Settings;
use betterlite_models::available_models::listing::ListingCredentials;
use betterlite_models::provider_metadata_factory::adapters::AzureFoundryModelInfo;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::anthropic_count::handler::FoundryCountHandler;
use crate::model_router as router;

const AZURE_HIDDEN: &str = r#"{"custom_llm_provider":"azure_ai"}"#;
const AZURE_SELECTED_HIDDEN: &str = r#"{"custom_llm_provider":"azure_ai","azure_model_router_selected_model":"azure_ai/provider-sent"}"#;
const SECOND_HIDDEN: &str = r#"{"custom_llm_provider":"second","other":1}"#;
const SECOND_SELECTED_HIDDEN: &str = r#"{"custom_llm_provider":"second","other":1,"azure_model_router_selected_model":"azure_ai/provider-second"}"#;
const ASSIGNED_SELECTED_HIDDEN: &str =
    r#"{"azure_model_router_selected_model":"azure_ai/deployment"}"#;
const STAMPED_HIDDEN: &str = r#"{"azure_model_router_selected_model":"stamped"}"#;
const EMPTY_STAMP_HIDDEN: &str = r#"{"azure_model_router_selected_model":""}"#;
const NULL_STAMP_HIDDEN: &str = r#"{"azure_model_router_selected_model":null}"#;
const NUMBER_STAMP_HIDDEN: &str = r#"{"azure_model_router_selected_model":7}"#;
const BOOL_STAMP_HIDDEN: &str = r#"{"azure_model_router_selected_model":true}"#;
const OBJECT_STAMP_HIDDEN: &str = r#"{"azure_model_router_selected_model":{}}"#;
const ROUTER_NAME_HIDDEN: &str = r#"{"litellm_model_name":"deployment/model_router/underlying"}"#;
const ROUTER_MODEL_HIDDEN: &str = r#"{"model":"model-router/hidden"}"#;
const NULL_NAME_ROUTER_MODEL_HIDDEN: &str =
    r#"{"litellm_model_name":null,"model":"model_router/hidden"}"#;
const NUMBER_NAME_ROUTER_MODEL_HIDDEN: &str =
    r#"{"litellm_model_name":7,"model":"model_router/hidden"}"#;
const PLAIN_NAME_HIDDEN: &str = r#"{"litellm_model_name":"plain-deployment"}"#;
const EMPTY_HIDDEN: &str = r#"{}"#;

#[rstest]
#[case::strip_model_router_prefix(strip_model_router_prefix)]
#[case::response_selected_model(response_selected_model)]
#[case::unknown_selected_model_fallback(unknown_selected_model_fallback)]
#[case::catalog_model_list(catalog_model_list)]
#[case::native_token_counter_selection(native_token_counter_selection)]
#[case::api_version_and_unsupported_validation(api_version_and_unsupported_validation)]
fn model_router_cases(#[case] check: fn()) {
    check();
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object parses")
}

fn response_body(model: Option<&str>) -> Json {
    let model_field = model.map_or(String::new(), |name| format!(r#""model":"{name}","#));
    let text = format!(
        r#"{{"id":"chatcmpl-fixture","object":"chat.completion","created":1700000000,{model_field}"choices":[{{"index":0,"message":{{"role":"assistant","content":"hi"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}}}"#
    );
    serde_json::from_str(&text).expect("the response body parses")
}

fn handler() -> FoundryCountHandler<()> {
    FoundryCountHandler {
        transport: (),
        request_timeout: Duration::from_secs(1),
        beta_version: String::from("2023-06-01"),
        configured_key: None,
    }
}

fn strip_model_router_prefix() {
    for (model, expected) in [
        ("deployment", "deployment"),
        ("model_router/deployment", "deployment"),
        ("model-router/deployment", "deployment"),
        ("azure_ai/model_router/deployment", "deployment"),
        ("prefix/model-router/name", "name"),
        (
            "prefix/model_router/name/model-router/rest",
            "name/model-router/rest",
        ),
        ("prefix/model-router/name/model_router/rest", "rest"),
        ("MODEL_ROUTER/deployment", "MODEL_ROUTER/deployment"),
        ("model_router/", ""),
        ("model-router/", ""),
        ("", ""),
    ] {
        assert_eq!(
            router::strip_model_router_prefix(model),
            expected,
            "{model}"
        );
    }
    let body = router::transform_request("azure_ai/model_router/deployment", &[], &Object::new())
        .expect("the request transforms");
    assert_eq!(body, object(r#"{"model":"deployment","messages":[]}"#));
    let routed = router::transform_response(
        "prefix/model-router/deployment",
        response_body(None),
        &Object::new(),
    )
    .expect("the response transforms");
    assert_eq!(routed.response.model, "azure_ai/deployment");
}

fn response_selected_model() {
    let first = router::transform_response(
        "model_router/deployment",
        response_body(Some("provider-sent")),
        &object(AZURE_HIDDEN),
    )
    .expect("the response transforms");
    assert_eq!(first.response.model, "azure_ai/provider-sent");
    assert_eq!(first.hidden_params, object(AZURE_SELECTED_HIDDEN));
    let second = router::transform_response(
        "model-router/second",
        response_body(Some("provider-second")),
        &object(SECOND_HIDDEN),
    )
    .expect("the second response transforms");
    assert_eq!(second.hidden_params, object(SECOND_SELECTED_HIDDEN));
    let assigned = router::transform_response(
        "model_router/deployment",
        response_body(None),
        &Object::new(),
    )
    .expect("the response without a model transforms");
    assert_eq!(assigned.response.model, "azure_ai/deployment");
    assert_eq!(assigned.hidden_params, object(ASSIGNED_SELECTED_HIDDEN));
    let mut empty_model = object(AZURE_HIDDEN);
    router::insert_selected_model(&mut empty_model, "");
    assert_eq!(empty_model, object(AZURE_HIDDEN));
}

fn unknown_selected_model_fallback() {
    for (stamp, expected) in [
        (STAMPED_HIDDEN, Some("stamped")),
        (EMPTY_STAMP_HIDDEN, None),
        (NULL_STAMP_HIDDEN, None),
        (NUMBER_STAMP_HIDDEN, None),
        (BOOL_STAMP_HIDDEN, None),
        (OBJECT_STAMP_HIDDEN, None),
        (EMPTY_HIDDEN, None),
    ] {
        let params = object(stamp);
        assert_eq!(router::selected_model(Some(&params)), expected, "{stamp}");
    }
    assert_eq!(router::selected_model(None), None);
    for (model, hidden_params, router_call) in [
        (None, Some(STAMPED_HIDDEN), true),
        (None, Some(ROUTER_NAME_HIDDEN), true),
        (None, Some(ROUTER_MODEL_HIDDEN), true),
        (None, Some(NULL_NAME_ROUTER_MODEL_HIDDEN), true),
        (None, Some(NUMBER_NAME_ROUTER_MODEL_HIDDEN), false),
        (Some("model_router/caller"), Some(PLAIN_NAME_HIDDEN), true),
        (Some("MODEL_ROUTER/caller"), None, true),
        (Some("agents/model_router/x"), None, false),
        (Some("plain-deployment"), Some(PLAIN_NAME_HIDDEN), false),
        (None, None, false),
    ] {
        let params = hidden_params.map(object);
        assert_eq!(
            router::is_model_router_call(model, params.as_ref()),
            router_call,
            "{model:?} {hidden_params:?}"
        );
    }
}

fn catalog_model_list() {
    let key = SecretString::from("explicit-key");
    let empty_key = SecretString::from("");
    for credentials in [
        ListingCredentials {
            api_key: None,
            api_base: None,
        },
        ListingCredentials {
            api_key: Some(&key),
            api_base: Some("https://resource.services.ai.azure.com"),
        },
        ListingCredentials {
            api_key: Some(&empty_key),
            api_base: Some(""),
        },
    ] {
        let models = router::get_models(credentials);
        assert!(models.is_empty());
    }
}

fn native_token_counter_selection() {
    for model in [
        None,
        Some(""),
        Some("gpt-4o"),
        Some("claude-3-5-sonnet"),
        Some("CLAUDE-3-opus"),
    ] {
        let info = AzureFoundryModelInfo {
            model: model.map(str::to_owned),
        };
        let counter = router::get_token_counter(&info, handler(), Settings::default());
        assert_eq!(
            counter.is_some(),
            model.is_some_and(|name| name.to_lowercase().contains("claude")),
            "{model:?}"
        );
    }
    let info = AzureFoundryModelInfo {
        model: Some(String::from("claude-3")),
    };
    let settings = Settings {
        api_base: Some(String::from("https://resource.example")),
        api_key: Some(SecretString::from("environment-key")),
    };
    let counter = router::get_token_counter(&info, handler(), settings)
        .expect("a claude model gets the native token counter");
    assert_eq!(
        counter.environment_api_base.as_deref(),
        Some("https://resource.example")
    );
    assert_eq!(
        counter
            .environment_api_key
            .as_ref()
            .map(|key| key.expose_secret()),
        Some("environment-key")
    );
}

fn api_version_and_unsupported_validation() {
    assert_eq!(
        router::get_api_version(Some("2024-01-01"), Some("2024-02-02"), Some("2024-03-03")),
        Some("2024-01-01")
    );
    assert_eq!(
        router::get_api_version(None, Some("2024-02-02"), Some("2024-03-03")),
        Some("2024-02-02")
    );
    assert_eq!(
        router::get_api_version(None, None, Some("2024-03-03")),
        Some("2024-03-03")
    );
    assert_eq!(router::get_api_version(None, None, None), None);
    assert_eq!(
        router::get_api_version(Some(""), Some("2024-02-02"), Some("2024-03-03")),
        Some("2024-02-02")
    );
    assert_eq!(
        router::get_api_version(Some(""), Some(""), Some("2024-03-03")),
        Some("2024-03-03")
    );
    assert_eq!(router::get_api_version(Some(""), Some(""), Some("")), None);
    let error = router::validate_environment().expect_err("validation always fails");
    assert!(matches!(
        error,
        router::ModelRouterError::ValidationUnsupported
    ));
    assert_eq!(
        error.to_string(),
        "Azure Foundry does not support environment validation"
    );
}
