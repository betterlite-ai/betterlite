use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::foundry_credentials::Settings;
use betterlite_foundry::anthropic_count::handler::FoundryCountHandler;
use betterlite_foundry::model_router::{
    get_api_version, get_models, get_token_counter, is_model_router_call, selected_model,
    strip_model_router_prefix, transform_request, transform_response, validate_environment,
};
use betterlite_models::available_models::listing::ListingCredentials;
use betterlite_models::provider_metadata_factory::adapters::AzureFoundryModelInfo;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::{CatalogCall, PropertyCall, ReaderCall, ResponseCall, StripCall, VersionCall};

pub(super) fn strip_report(call: &StripCall) -> Value {
    let stripped = strip_model_router_prefix(&call.model);
    let Ok(body) = transform_request(&call.model, &[], &Object::new()) else {
        return json!({"stripped": stripped, "error": {"kind": "RequestError"}});
    };
    json!({"stripped": stripped, "body": body})
}

pub(super) fn response_report(call: &ResponseCall) -> Value {
    let Ok(body) = serde_json::from_str::<Json>(&call.body) else {
        return json!({"error": {"kind": "JSONDecodeError"}});
    };
    let Ok(routed) = transform_response(&call.model, body, &Object::new()) else {
        return json!({"error": {"kind": "ResponseError"}});
    };
    json!({
        "model": routed.response.model,
        "selected": selected_model(Some(&routed.hidden_params)),
    })
}

pub(super) fn reader_report(call: &ReaderCall) -> Value {
    json!({
        "selected": selected_model(call.hidden_params.as_ref()),
        "is_call": is_model_router_call(call.model.as_deref(), call.hidden_params.as_ref()),
    })
}

pub(super) fn catalog_report(call: &CatalogCall) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    let credentials = ListingCredentials {
        api_key: key.as_ref(),
        api_base: call.api_base.as_deref(),
    };
    json!({"models": get_models(credentials)})
}

pub(super) fn token_report(model: Option<&str>) -> Value {
    let info = AzureFoundryModelInfo {
        model: model.map(str::to_owned),
    };
    let counter = get_token_counter(&info, handler(), Settings::default());
    json!(counter.is_some())
}

pub(super) fn version_report(call: &VersionCall) -> Value {
    json!({
        "api_version": get_api_version(
            call.api_version.as_deref(),
            call.global_api_version.as_deref(),
            version_environment(&call.environment),
        ),
    })
}

pub(super) fn property_report(call: &PropertyCall) -> Value {
    json!({
        "api_version": get_api_version(
            None,
            call.global_api_version.as_deref(),
            version_environment(&call.environment),
        ),
    })
}

pub(super) fn validate_report() -> Value {
    let message = match validate_environment() {
        Ok(never) => match never {},
        Err(error) => error.to_string(),
    };
    json!({"kind": "NotImplementedError", "message": message})
}

fn handler() -> FoundryCountHandler<()> {
    FoundryCountHandler {
        transport: (),
        request_timeout: Duration::from_secs(1),
        beta_version: String::from("2023-06-01"),
        configured_key: None,
    }
}

// an empty variable counts as unset
fn version_environment(environment: &BTreeMap<String, String>) -> Option<&str> {
    environment
        .get("AZURE_API_VERSION")
        .filter(|value| !value.is_empty())
        .map(String::as_str)
}
