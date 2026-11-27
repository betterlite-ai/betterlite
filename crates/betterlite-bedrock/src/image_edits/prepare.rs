use std::collections::BTreeMap;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::error::ImageEditError;
use super::plan::{EditCall, EditPlan, PreparedRequest};
use super::request::request_body;
use crate::aws_endpoints::model_id::bedrock_model_id;
use crate::aws_endpoints::provider::invoke_provider;
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::aws_settings::{AUTHENTICATION_PARAMS, bedrock_bearer_token};
use crate::image_call::plan::{CredentialSource, LogContext};
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::headers::set_exact;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::{RegionInputs, RequestTarget, TargetRequest, request_target};

// LiteLLM resolves the credential target before anything reads the body, and pops the auth keys out.
pub async fn resolve_target<Source>(
    call: &mut EditCall<'_>,
    source: &Source,
) -> Result<RequestTarget, ImageEditError>
where
    Source: CredentialSource + Sync,
{
    let mut raw = BTreeMap::new();
    for (key, value) in &call.optional_params {
        raw.insert(
            key.clone(),
            RawValue::from_string(serde_json::to_string(value)?)?,
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

// The URL and model id are settled before the body is built, as LiteLLM's _prepare_request orders it.
pub fn prepare_request(
    call: &mut EditCall<'_>,
    plan: &EditPlan<'_, '_>,
    target: &RequestTarget,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRequest, ImageEditError> {
    let endpoint_url = request_url(call, target)?;
    let data = request_body(call, plan)?;
    let headers = base_headers(call.extra_headers);
    let signed = sign_payload(call, target, &endpoint_url, &data, &headers)?;
    log_pre_call(call.prompt, &endpoint_url, &signed, &data, log)?;
    Ok(PreparedRequest {
        endpoint_url,
        signed,
        data,
    })
}

fn request_url(call: &mut EditCall<'_>, target: &RequestTarget) -> Result<String, ImageEditError> {
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
    call: &EditCall<'_>,
    target: &RequestTarget,
    endpoint_url: &str,
    data: &ImageParams,
    headers: &[(String, Option<String>)],
) -> Result<AwsSignedRequest, ImageEditError> {
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

// Python merges the extra headers into a fresh Content-Type dict; an existing exact name is replaced.
fn base_headers(extra: Option<&[(String, String)]>) -> Vec<(String, Option<String>)> {
    let mut headers = vec![(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    for (name, value) in extra.unwrap_or_default() {
        set_exact(&mut headers, name, value.clone());
    }
    headers
}

// B249 records a masked copy of the headers, and an empty api_key never reaches a log.
fn log_pre_call(
    prompt: Option<&str>,
    api_base: &str,
    signed: &AwsSignedRequest,
    data: &ImageParams,
    log: &mut LogContext<'_, '_>,
) -> Result<(), ImageEditError> {
    let mut headers = IndexMap::new();
    for (name, value) in &signed.headers {
        headers.insert(name.clone(), Json::Text(value.clone()));
    }
    let input = prompt.map_or_else(|| Json::Null, |text| Json::Text(text.to_owned()));
    pre_call(
        log.state,
        &PreCall {
            input,
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
