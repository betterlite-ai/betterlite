use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use serde_json::value::RawValue;

use super::error::ImageCallError;
use crate::aws_endpoints::model_id::bedrock_model_id;
use crate::aws_endpoints::provider::invoke_provider;
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::aws_settings::sources::Credentials;
use crate::aws_settings::{AUTHENTICATION_PARAMS, bedrock_bearer_token};
use crate::credentials::error::CredentialError;
use crate::image_models::family::{Family, select_family};
use crate::image_models::params::{Field, Params};
use crate::image_models::stability1::ClassSettings;
use crate::image_models::{nova, stability1, stability3, titan};
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::{RegionInputs, RequestTarget, TargetRequest, request_target};

#[derive(Debug, Clone, Copy, Default)]
pub struct ImageEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
    pub session_region: Option<&'text str>,
    pub runtime_endpoint: Option<&'text str>,
    pub bearer_token: Option<&'text SecretString>,
}

pub struct ImageCall<'text> {
    pub model: &'text str,
    pub prompt: &'text str,
    pub optional_params: Params,
    pub api_base: Option<&'text str>,
    pub extra_headers: Option<&'text [(String, String)]>,
    pub api_key: Option<&'text SecretString>,
    pub timeout: Option<Duration>,
    pub aimg_generation: bool,
    pub environment: ImageEnvironment<'text>,
    pub now: SystemTime,
}

pub struct PreparedRequest {
    pub endpoint_url: String,
    pub signed: AwsSignedRequest,
    pub data: IndexMap<String, Json>,
}

pub trait CredentialSource {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send;
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub async fn resolve_target<Source>(
    call: &mut ImageCall<'_>,
    source: &Source,
) -> Result<RequestTarget, ImageCallError>
where
    Source: CredentialSource + Sync,
{
    let mut raw = BTreeMap::new();
    for (key, field) in &call.optional_params {
        raw.insert(
            key.clone(),
            RawValue::from_string(serde_json::to_string(field)?)?,
        );
    }
    let bearer = bedrock_bearer_token(call.api_key, call.environment.bearer_token);
    let target = request_target(
        TargetRequest {
            optional_params: &mut raw,
            model: Some(call.model),
            region_inputs: RegionInputs {
                environment_region_name: call.environment.region_name,
                environment_region: call.environment.region,
                session_region: call.environment.session_region,
            },
            bearer_token: bearer.as_ref(),
        },
        |settings, region| source.resolve(settings.clone(), region.to_owned()),
    )
    .await?;
    for key in AUTHENTICATION_PARAMS {
        call.optional_params.shift_remove(key);
    }
    Ok(target)
}

pub fn prepare_request(
    call: &mut ImageCall<'_>,
    target: &RequestTarget,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRequest, ImageCallError> {
    let endpoint_url = request_url(call, target)?;
    let body = build_body(call)?;
    let data = to_json_object(&body);
    let mut headers = base_headers(call.extra_headers);
    for (name, value) in guardrail_headers(&mut call.optional_params)? {
        set_header(&mut headers, &name, value);
    }
    let signed = sign_payload(call, target, &endpoint_url, &data, &headers)?;
    log_pre_call(call.prompt, &endpoint_url, &signed, &data, log)?;
    Ok(PreparedRequest {
        endpoint_url,
        signed,
        data,
    })
}

fn request_url(call: &mut ImageCall<'_>, target: &RequestTarget) -> Result<String, ImageCallError> {
    let provider = invoke_provider(call.model);
    let mut raw = BTreeMap::new();
    if let Some(field) = call.optional_params.shift_remove("model_id") {
        raw.insert(
            "model_id".to_owned(),
            RawValue::from_string(serde_json::to_string(&field)?)?,
        );
    }
    let model_id = bedrock_model_id(&mut raw, provider, call.model)?;
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: call.api_base,
        configured_endpoint: target.endpoint.as_deref(),
        environment_endpoint: call.environment.runtime_endpoint,
        region: &target.region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    Ok(format!(
        "{}/model/{model_id}/invoke",
        endpoint.proxy_endpoint_url
    ))
}

fn sign_payload(
    call: &ImageCall<'_>,
    target: &RequestTarget,
    endpoint_url: &str,
    data: &IndexMap<String, Json>,
    headers: &[(String, Option<String>)],
) -> Result<AwsSignedRequest, ImageCallError> {
    let body = dumps_object(data).into_bytes();
    Ok(request_headers(RequestHeaders {
        credentials: target.credentials.as_ref(),
        region: &target.region,
        extra_headers: call.extra_headers.unwrap_or_default(),
        endpoint: endpoint_url,
        data: Body::Bytes(&body),
        headers,
        api_key: call.api_key,
        configured_bearer: call.environment.bearer_token,
        supports_bearer_token: true,
        now: call.now,
    })?)
}

fn build_body(call: &ImageCall<'_>) -> Result<Params, ImageCallError> {
    let params = call.optional_params.clone();
    match select_family(Some(call.model)) {
        Family::Titan => Ok(titan::transform_request_body(call.prompt, params)?),
        Family::Stability1 => Ok(stability1::transform_request_body(
            call.prompt,
            &params,
            &ClassSettings::default(),
        )),
        Family::Stability3 => Ok(stability3::transform_request_body(call.prompt, params)?),
        Family::Nova => Ok(nova::transform_request_body(call.prompt, params)?),
    }
}

fn base_headers(extra: Option<&[(String, String)]>) -> Vec<(String, Option<String>)> {
    let mut headers = vec![(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    for (name, value) in extra.unwrap_or_default() {
        set_header(&mut headers, name, value.clone());
    }
    headers
}

fn set_header(headers: &mut Vec<(String, Option<String>)>, name: &str, value: String) {
    match headers.iter_mut().find(|(existing, _)| existing == name) {
        Some((_, stored)) => *stored = Some(value),
        None => headers.push((name.to_owned(), Some(value))),
    }
}

fn guardrail_headers(params: &mut Params) -> Result<Vec<(String, String)>, ImageCallError> {
    let mut headers = Vec::new();
    for (key, name) in [
        ("guardrailIdentifier", "x-amz-bedrock-guardrail-identifier"),
        ("guardrailVersion", "x-amz-bedrock-guardrail-version"),
    ] {
        match params.shift_remove(key) {
            None | Some(Field::Null) => {}
            Some(Field::Text(value)) => headers.push((name.to_owned(), value)),
            Some(_) => return Err(ImageCallError::GuardrailType(key)),
        }
    }
    Ok(headers)
}

// B249 records a masked copy of the headers, and an empty api_key never reaches a log
fn log_pre_call(
    prompt: &str,
    api_base: &str,
    signed: &AwsSignedRequest,
    data: &IndexMap<String, Json>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), ImageCallError> {
    let mut headers = IndexMap::new();
    for (name, value) in &signed.headers {
        headers.insert(name.clone(), Json::Text(value.clone()));
    }
    pre_call(
        log.state,
        &PreCall {
            input: Json::Text(prompt.to_owned()),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(api_base.to_owned()),
                headers: Some(headers),
                complete_input_dict: Some(RawBody::Object(data.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    )?;
    Ok(())
}

fn to_json_object(params: &Params) -> IndexMap<String, Json> {
    params
        .iter()
        .map(|(key, field)| (key.clone(), field_json(field)))
        .collect()
}

fn field_json(field: &Field) -> Json {
    match field {
        Field::Null => Json::Null,
        Field::Bool(flag) => Json::Bool(*flag),
        Field::Int(value) => Json::Number(Number::from(*value)),
        Field::Float(value) => Number::from_f64(*value).map_or(Json::Null, Json::Number),
        Field::Text(text) => Json::Text(text.clone()),
        Field::List(items) => Json::List(items.iter().map(field_json).collect()),
        Field::Mapping(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), field_json(value)))
                .collect(),
        ),
    }
}
