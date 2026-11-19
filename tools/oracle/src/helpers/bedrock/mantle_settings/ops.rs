use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::mantle_inputs::chat_endpoint::{ChatEndpointParams, chat_endpoint_inputs};
use betterlite_bedrock::mantle_inputs::records::MantleGlobals;
use betterlite_bedrock::mantle_inputs::region::resolve_region;
use betterlite_bedrock::mantle_settings::auth::{SignRequest, sign_request};
use betterlite_bedrock::mantle_settings::chat::{CUSTOM_LLM_PROVIDER, MantleChatConfig};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::fixtures::{self, Policy};
use super::input::{BaseCase, Catalog, ChatCase, ErrorProbe, Input, SignCase};
use super::outcomes;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

pub(super) fn globals(environment: &BTreeMap<String, String>) -> MantleGlobals {
    MantleGlobals {
        mantle_api_key: environment
            .get("BEDROCK_MANTLE_API_KEY")
            .cloned()
            .map(SecretString::from),
        mantle_api_base: environment.get("BEDROCK_MANTLE_API_BASE").cloned(),
        mantle_region: environment.get("BEDROCK_MANTLE_REGION").cloned(),
        aws_bearer_token: environment
            .get("AWS_BEARER_TOKEN_BEDROCK")
            .cloned()
            .map(SecretString::from),
        aws_region_name: environment.get("AWS_REGION_NAME").cloned(),
        aws_region: environment.get("AWS_REGION").cloned(),
    }
}

pub(super) fn object_with_region(region: Option<&str>) -> Object {
    let mut params = Object::new();
    if let Some(region) = region {
        params.insert("aws_region_name".to_owned(), json!(region));
    }
    params
}

fn chat_value(
    case: &BaseCase,
    api_key: Option<&SecretString>,
    globals: &MantleGlobals,
    model_cost: &ParamMap,
) -> Value {
    let params = ChatEndpointParams {
        model: case.model.as_deref(),
        api_base: case.api_base.as_deref(),
        api_key,
        aws_region_name: case.aws_region_name.as_deref(),
    };
    chat_endpoint_inputs(params, globals, model_cost).map_or_else(
        |_error| json!({"error": true}),
        |resolved| {
            json!({
                "api_base": resolved.api_base,
                "api_key": resolved.api_key.map(|key| key.expose_secret().to_owned()),
            })
        },
    )
}

fn standalone_value(case: &BaseCase, globals: &MantleGlobals) -> Value {
    resolve_region(
        case.aws_region_name.as_deref(),
        case.api_base.as_deref(),
        globals,
    )
    .map_or_else(
        |_error| json!({"error": true}),
        |region| json!({"region": region}),
    )
}

fn base_cases(model_cost: &ParamMap, cases: &[BaseCase]) -> HelperResult {
    let cache = fixtures::credential_cache()?;
    let mut chat = Vec::with_capacity(cases.len());
    let mut standalone = Vec::with_capacity(cases.len());
    let mut signers = Vec::with_capacity(cases.len());
    for case in cases {
        let globals = globals(&case.environment);
        let api_key = case.api_key.as_deref().map(SecretString::from);
        chat.push(chat_value(case, api_key.as_ref(), &globals, model_cost));
        standalone.push(standalone_value(case, &globals));
        let injected = case.inject_signer.then(|| Arc::clone(&cache));
        let config = MantleChatConfig::new(injected);
        if config.credential_cache.is_some() {
            signers.push("injected");
        } else {
            signers.push("default");
        }
    }
    let config = MantleChatConfig::new(None);
    Ok(json!({
        "provider": <&'static str>::from(CUSTOM_LLM_PROVIDER),
        "get_config": serde_json::to_value(config.get_config())?,
        "signers": signers,
        "chat": chat,
        "standalone": standalone,
    }))
}

fn scope(value: &str) -> &str {
    if value.starts_with("Bearer ") {
        return value;
    }
    value.split_once(',').map_or(value, |(head, _rest)| head)
}

fn signed_value(signed: AwsSignedRequest) -> Value {
    let mut headers = Object::new();
    let mut authorization = Value::Null;
    for (name, value) in &signed.headers {
        if name.eq_ignore_ascii_case("authorization") {
            authorization = json!(scope(value));
        } else {
            headers.insert(name.to_ascii_lowercase(), json!(value));
        }
    }
    let body = serde_json::from_slice::<Value>(&signed.body).unwrap_or(Value::Null);
    json!({
        "url": signed.url,
        "method": signed.method,
        "headers": headers,
        "body": body,
        "authorization": authorization,
    })
}

fn auth_cases(cases: &[SignCase]) -> HelperResult {
    let runtime = Builder::new_current_thread().build()?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let globals = globals(&case.environment);
        let api_key = case.api_key.as_deref().map(SecretString::from);
        let credentials = fixtures::credentials(&case.credentials);
        let seconds = u64::try_from(case.now).or_value_error()?;
        let now = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .ok_or("now is out of range")
            .or_value_error()?;
        let request = SignRequest {
            headers: &case.headers,
            optional_params: &case.optional_params,
            request_data: &case.request_data,
            api_base: &case.api_base,
            api_key: api_key.as_ref(),
            model: case.model.as_deref(),
            stream: case.stream,
            fake_stream: case.fake_stream,
            globals: &globals,
            now,
        };
        let signed = runtime.block_on(sign_request(request, move |_settings, _region| {
            async move { Ok::<Credentials, CredentialError>(credentials) }
        }));
        results.push(signed.map_or_else(|_error| json!({"error": true}), signed_value));
    }
    Ok(json!({"results": results}))
}

fn error_probe(config: &MantleChatConfig, probe: &ErrorProbe) -> Result<Value, Box<HelperError>> {
    let error = config
        .get_error_class(&probe.message, probe.status_code, &probe.headers)
        .map_err(fixtures::failure)?;
    let headers: BTreeMap<String, String> = error
        .error
        .headers
        .unwrap_or_default()
        .into_iter()
        .collect();
    Ok(json!({
        "status": error.error.status_code,
        "message": error.error.message,
        "headers": headers,
    }))
}

fn chat_cases(catalog: &Catalog, cases: &[ChatCase]) -> HelperResult {
    let runtime = Builder::new_current_thread().build()?;
    let capabilities = CatalogCapabilities {
        chat_models: catalog.chat_models.iter().cloned().collect(),
        text_models: catalog.text_models.iter().cloned().collect(),
    };
    let config = MantleChatConfig::new(None);
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut policy = Policy::new(case.reasoning);
        let params =
            runtime.block_on(config.supported_params(&case.model, &capabilities, &mut policy));
        let api_key = case.api_key.as_deref().map(SecretString::from);
        let headers = config.validate_environment(
            api_key.as_ref(),
            &case.headers,
            case.project_id.as_deref(),
        );
        results.push(json!({
            "headers": headers,
            "params": params,
            "requests": policy.seen.take(),
            "error": error_probe(&config, &case.error)?,
            "json_mode": MantleChatConfig::response_iterator(case.sync_stream, case.json_mode).json_mode,
        }));
    }
    Ok(json!({"results": results}))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Base { model_cost, cases } => base_cases(model_cost, cases),
        Input::Auth { cases } => auth_cases(cases),
        Input::Chat { catalog, cases } => chat_cases(catalog, cases),
        Input::Messages { .. }
        | Input::Passthrough { .. }
        | Input::Failure { .. }
        | Input::ReplyShape { .. } => outcomes::run(input),
    }
}
