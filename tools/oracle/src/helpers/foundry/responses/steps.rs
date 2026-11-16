use betterlite_config::sections::foundry_credentials::Settings;
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::responses::auth::{EnvironmentRequest, validate_environment};
use betterlite_foundry::responses::decisions::AzureAiDecisionsConfig;
use betterlite_foundry::responses::url::complete_url;
use betterlite_foundry::responses::{PROVIDER, SUPPORTS_NATIVE_WEBSOCKET};
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::fixture::EchoSources;
use super::{AuthStep, BaseStep, DecisionsStep, Step};

pub(super) fn step_answer(step: &Step) -> Value {
    match step {
        Step::Auth(auth) => auth_answer(auth),
        Step::DecisionsRoute(decisions) => decisions_answer(decisions),
        Step::MissingBase(base) => url_answer("missing_base", base),
        Step::NoNativeWebsocket => json!({
            "kind": "no_native_websocket",
            "provider": PROVIDER.to_string(),
            "websocket": SUPPORTS_NATIVE_WEBSOCKET,
        }),
        Step::Url(base) => url_answer("url", base),
    }
}

fn url_answer(kind: &str, step: &BaseStep) -> Value {
    // the case's environment is the AZURE_AI_* layer, as B549 reads it in production
    let settings = Settings::resolve_environment(&step.environment);
    let accessors = ModelInfoAccessors {
        global_api_base: step.global_api_base.as_deref(),
        global_api_key: None,
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: None,
    };
    complete_url(accessors, step.api_base.as_deref()).map_or_else(
        |_error| json!({"kind": "error"}),
        |url| json!({"kind": kind, "url": url}),
    )
}

fn auth_answer(step: &AuthStep) -> Value {
    let settings = Settings::resolve_environment(&step.environment);
    let global_api_key = step.global_api_key.clone().map(SecretString::from);
    let api_key = step.api_key.clone().map(SecretString::from);
    let accessors = ModelInfoAccessors {
        global_api_base: step.global_api_base.as_deref(),
        global_api_key: global_api_key.as_ref(),
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: settings.api_key.as_ref(),
    };
    let sources = EchoSources;
    let validated = validate_environment(&EnvironmentRequest {
        headers: &step.headers,
        accessors,
        api_base: step.api_base.as_deref(),
        api_key: api_key.as_ref(),
        params: step.params.as_ref(),
        environment: &step.environment,
        refresh_enabled: false,
        sources: &sources,
    });
    validated.map_or_else(
        |_error| json!({"kind": "error"}),
        |headers| json!({"kind": "auth", "headers": headers}),
    )
}

fn decisions_answer(step: &DecisionsStep) -> Value {
    let config = AzureAiDecisionsConfig;
    let url = step
        .api_base
        .as_deref()
        .map(|api_base| config.get_complete_url(api_base, &step.model));
    // the trait asks for a plain fn pointer, so the case's environment values never reach these two resolutions
    json!({
        "kind": "decisions_route",
        "url": url,
        "api_base": config.resolve_api_base(step.api_base.as_deref(), |_name| String::new()),
        "api_key": config.resolve_api_key(step.api_key.as_deref(), |_name| String::new()),
    })
}
