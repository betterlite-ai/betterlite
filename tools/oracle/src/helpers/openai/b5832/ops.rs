use betterlite_openai::fireworks_ai_common::FireworksEnv;
use betterlite_openai::fireworks_ai_responses::delete::transform_delete_response;
use betterlite_openai::fireworks_ai_responses::headers::validate_environment;
use betterlite_openai::fireworks_ai_responses::request::transform_responses_request;
use betterlite_openai::fireworks_ai_responses::settings::{FireworksResponsesEnv, SessionSource};
use betterlite_openai::fireworks_ai_responses::url::get_complete_url;
use betterlite_openai::fireworks_ai_responses::{
    PROVIDER, SUPPORTS_NATIVE_FILE_SEARCH, SUPPORTS_NATIVE_WEBSOCKET,
};
use betterlite_openai::fireworks_ai_text_completion::extra_body::map_extra_body_params;
use betterlite_openai::fireworks_ai_text_completion::mapping::map_openai_params;
use betterlite_openai::fireworks_ai_text_completion::request::transform_completion_request;
use betterlite_openai::fireworks_ai_text_completion::supported::SUPPORTED_OPENAI_PARAMS;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object as Params;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::Fixture;
use super::input::{EnvCall, Input, SessionCall, Step};

pub(crate) fn run(input: &Input) -> HelperResult {
    let mut fixture = Fixture::load(&input.catalog);
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| failure(source.to_string()))?;
    runtime.block_on(execute(input, &mut fixture))
}

async fn execute(input: &Input, fixture: &mut Fixture) -> HelperResult {
    let mut outcomes = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        outcomes.push(step_value(step, fixture).await?);
    }
    Ok(Value::Array(outcomes))
}

async fn step_value(step: &Step, fixture: &mut Fixture) -> HelperResult {
    match step {
        Step::Supported => Ok(json!(SUPPORTED_OPENAI_PARAMS)),
        Step::Capability => Ok(capabilities()),
        Step::MapParams {
            non_default_params,
            optional_params,
        } => Ok(mapped_params(non_default_params, optional_params)),
        Step::ExtraBody {
            model,
            optional_params,
        } => Ok(Value::Object(
            map_extra_body_params(fixture, model, optional_params).await,
        )),
        Step::Completion {
            model,
            messages,
            optional_params,
        } => completion_outcome(fixture, model, messages, optional_params).await,
        Step::Headers {
            headers,
            api_key,
            env,
            session,
        } => authorized_headers(headers, api_key.as_deref(), env, session),
        Step::Url { api_base, env } => Ok(Value::String(get_complete_url(
            api_base.as_deref(),
            &responses_env(env),
        ))),
        Step::Transform {
            model,
            input,
            optional_params,
        } => transformed_body(model, input, optional_params),
        Step::Delete { path } => deleted_result(path),
    }
}

fn capabilities() -> Value {
    json!({
        "provider": PROVIDER.to_string(),
        "native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
        "native_file_search": SUPPORTS_NATIVE_FILE_SEARCH,
    })
}

fn mapped_params(non_default_params: &Object, optional_params: &Object) -> Value {
    let mut optional = optional_params.clone();
    map_openai_params(&pairs(non_default_params), &mut optional);
    Value::Object(optional)
}

async fn completion_outcome(
    fixture: &mut Fixture,
    model: &str,
    messages: &[Value],
    optional_params: &Object,
) -> HelperResult {
    match transform_completion_request(fixture, model, messages, optional_params).await {
        Ok(request) => Ok(Value::Object(request)),
        Err(error) => Ok(json!({"raised": error.status_code()})),
    }
}

fn authorized_headers(
    headers: &Headers,
    api_key: Option<&str>,
    env: &EnvCall,
    session: &SessionCall,
) -> HelperResult {
    let key = api_key.map(SecretString::from);
    let source = SessionSource {
        litellm_session_id: session.litellm_session_id.clone(),
        litellm_metadata: session.litellm_metadata.clone(),
    };
    match validate_environment(headers, key.as_ref(), &source, &responses_env(env)) {
        Ok(authorized) => Ok(serde_json::to_value(&authorized)?),
        Err(_) => Ok(json!({"raised": null})),
    }
}

fn transformed_body(model: &str, input: &ResponsesInput, optional_params: &Params) -> HelperResult {
    match transform_responses_request(model, input, optional_params, None) {
        Ok(body) => Ok(serde_json::to_value(&body)?),
        Err(error) => Ok(json!({"raised": error.status_code()})),
    }
}

fn deleted_result(path: &str) -> HelperResult {
    match transform_delete_response(path) {
        Ok(deleted) => Ok(serde_json::to_value(&deleted)?),
        Err(error) => Ok(json!({"raised": error.status_code()})),
    }
}

fn pairs(params: &Object) -> Vec<(String, Value)> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn responses_env(env: &EnvCall) -> FireworksResponsesEnv {
    FireworksResponsesEnv {
        keys: FireworksEnv {
            fireworks_api_key: env.fireworks_api_key.as_deref().map(SecretString::from),
            fireworks_ai_api_key: env.fireworks_ai_api_key.as_deref().map(SecretString::from),
            fireworksai_api_key: env.fireworksai_api_key.as_deref().map(SecretString::from),
            fireworks_ai_token: env.fireworks_ai_token.as_deref().map(SecretString::from),
        },
        api_base: env.api_base.clone(),
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("FixtureError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
