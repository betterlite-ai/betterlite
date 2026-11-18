use std::collections::BTreeMap;

use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::anthropic_beta_filter::Headers;
use betterlite_core::api::anthropic_beta_filter::matrix_filter::filter_header_betas;
use betterlite_http::user_url::validate::Resolve;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use crate::aws_endpoints::region::DEFAULT_AWS_REGION;
use crate::aws_settings::bedrock_bearer_token;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_response::environment::should_fake_stream;
use crate::converse_response::request::OptionalParams;
use crate::event_frames::beta::BetaHeader;
use crate::model_routes::{ParamMap, ParamValue};
use crate::rerank::plan::CredentialSource;
use crate::signing::body::Body;
use crate::signing::endpoint::RequestHeaders;
use crate::signing::request::AwsSignedRequest;

use super::ConverseAssembly;
use super::ConverseCall;
use super::ConverseEnvironment;
use super::error::ConverseCallError;
use super::request::{BodyRequest, PreparedBody, build_body};
use super::target::{PreparedTarget, TargetInput, converse_model_id, resolve_target};
use super::transport::ConverseSigner;

const CONVERSE_PROVIDER: &str = "bedrock_converse";
const REGION_FIELD: &str = "aws_region_name";

pub(super) struct ExtractInput<'call> {
    pub(super) params: &'call mut ParamMap,
    pub(super) model: &'call str,
    pub(super) extra_headers: Option<&'call [(String, String)]>,
    pub(super) control: ControlOptions,
    pub(super) api_key: Option<&'call SecretString>,
    pub(super) environment: ConverseEnvironment<'call>,
}

pub(super) struct Prepared {
    pub(super) stream: Option<bool>,
    pub(super) fake_stream: bool,
    pub(super) model_id: String,
    region_model: String,
    pub(super) bearer: Option<SecretString>,
    headers: Headers,
    pub(super) stream_chunk_size: Option<u64>,
}

pub(super) struct CallPlan {
    pub(super) url: String,
    pub(super) signed: AwsSignedRequest,
    pub(super) data: String,
    pub(super) messages: Json,
    pub(super) optional: OptionalParams,
    pub(super) tool_name_mappings: ToolNameMap,
    pub(super) stream: Option<bool>,
    pub(super) fake_stream: bool,
    pub(super) json_mode: Option<bool>,
    pub(super) stream_chunk_size: Option<u64>,
}

// completion()'s setup: the popped routing switches, the path id and region derived from the model,
// the bearer that replaces credentials, and the beta-filtered headers signing sees.
pub(super) fn extract(
    input: ExtractInput<'_>,
    betas: &BetaMatrix,
) -> Result<Prepared, ConverseCallError> {
    let stream = pop_bool(input.params, "stream")?;
    let model_id = pop_text(input.params, "model_id")?;
    let fake_stream = pop_bool(input.params, "fake_stream")?;
    let derived = converse_model_id(input.model, model_id.as_deref());
    if let Some(region) = derived.region
        && !input.params.contains_key(REGION_FIELD)
    {
        input
            .params
            .insert(REGION_FIELD.to_owned(), ParamValue::Text(region));
    }
    let bearer = bedrock_bearer_token(input.api_key, input.environment.bearer_token);
    let mut headers = merged_headers(input.extra_headers);
    filter_header_betas(&mut headers, betas, CONVERSE_PROVIDER)?;
    let stream_chunk_size = if stream == Some(true) {
        input.control.stream_chunk_size
    } else {
        None
    };
    Ok(Prepared {
        stream,
        fake_stream: should_fake_stream(Some(input.model), stream, fake_stream),
        model_id: derived.model_id,
        region_model: model_id.unwrap_or_else(|| input.model.to_owned()),
        bearer,
        headers,
        stream_chunk_size,
    })
}

// The region depends on the original model or the explicit id, the URL on the encoded path id; the
// body follows the target, so the auth params and endpoint are gone before it is transformed.
pub(super) async fn prepare<Cred, Source, Transport, Lookup, R, Ids>(
    call: &mut ConverseCall<'_>,
    prepared: Prepared,
    signer: &dyn ConverseSigner,
    credentials: &Cred,
    assembly: ConverseAssembly<'_, '_, Source, Transport, Lookup, R, Ids>,
) -> Result<CallPlan, ConverseCallError>
where
    Cred: CredentialSource + Sync,
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let target = resolve_target(
        &TargetInput {
            region_model: &prepared.region_model,
            model_id: &prepared.model_id,
            stream_path: prepared.stream == Some(true) && !prepared.fake_stream,
            bearer: prepared.bearer.as_ref(),
            api_base: call.api_base,
            environment: call.environment,
        },
        &mut call.optional_params,
        &mut call.litellm_params,
        credentials,
    )
    .await?;
    let body = transformed_body(call, &prepared, assembly).await?;
    let signed = sign_payload(call, &prepared, &target, &body.data, signer)?;
    let json_mode = body.optional.json_mode;
    Ok(CallPlan {
        url: target.url,
        signed,
        data: body.data,
        messages: messages_json(call.messages)?,
        json_mode,
        optional: body.optional,
        tool_name_mappings: body.tool_name_mappings,
        stream: prepared.stream,
        fake_stream: prepared.fake_stream,
        stream_chunk_size: prepared.stream_chunk_size,
    })
}

async fn transformed_body<Source, Transport, Lookup, R, Ids>(
    call: &mut ConverseCall<'_>,
    prepared: &Prepared,
    assembly: ConverseAssembly<'_, '_, Source, Transport, Lookup, R, Ids>,
) -> Result<PreparedBody, ConverseCallError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let ConverseAssembly {
        environment,
        lookup,
        media,
        model_cost,
        catalogs,
        packaged,
    } = assembly;
    let headers = transform_headers(call, prepared);
    Ok(build_body(
        BodyRequest {
            model: call.model,
            messages: call.messages,
            params: &mut call.optional_params,
            litellm_params: &mut call.litellm_params,
            headers: headers.as_ref(),
            settings: call.settings,
            model_cost,
            catalogs,
            packaged,
        },
        lookup,
        media,
        environment,
    )
    .await?)
}

// Async transforms with the filtered merged headers; sync hands the transform the original extras.
fn transform_headers(
    call: &ConverseCall<'_>,
    prepared: &Prepared,
) -> Option<BTreeMap<String, BetaHeader>> {
    if call.acompletion {
        return Some(beta_headers(prepared.headers.iter()));
    }
    call.extra_headers
        .map(|pairs| beta_headers(pairs.iter().map(|(name, value)| (name, value))))
}

// Async signs the filtered merged headers as both sources and reads the stored region; sync signs
// with the resolved region, the merged headers and the original extras as its override source.
fn sign_payload(
    call: &ConverseCall<'_>,
    prepared: &Prepared,
    target: &PreparedTarget,
    data: &str,
    signer: &dyn ConverseSigner,
) -> Result<AwsSignedRequest, ConverseCallError> {
    let (extra_headers, headers) = signed_header_sources(call, prepared);
    let region = signing_region(call, target);
    Ok(signer.sign(RequestHeaders {
        credentials: target.credentials.as_ref(),
        region: &region,
        extra_headers: &extra_headers,
        endpoint: &target.url,
        data: Body::Text(data),
        headers: &headers,
        api_key: call.api_key,
        configured_bearer: call.environment.bearer_token,
        supports_bearer_token: true,
        now: call.now,
    })?)
}

fn signed_header_sources(
    call: &ConverseCall<'_>,
    prepared: &Prepared,
) -> (Vec<(String, String)>, Vec<(String, Option<String>)>) {
    let extra_headers: Vec<(String, String)> = if call.acompletion {
        prepared
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    } else {
        call.extra_headers.unwrap_or_default().to_vec()
    };
    let headers = prepared
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    (extra_headers, headers)
}

fn signing_region(call: &ConverseCall<'_>, target: &PreparedTarget) -> String {
    if call.acompletion {
        return stored_region(&call.litellm_params);
    }
    target.region.clone()
}

fn stored_region(params: &PromptObject) -> String {
    params
        .get(REGION_FIELD)
        .and_then(PromptJson::as_str)
        .filter(|region| !region.is_empty())
        .map_or_else(|| DEFAULT_AWS_REGION.to_owned(), str::to_owned)
}

// {"Content-Type": "application/json", **extra_headers}: a same-case caller key replaces the value
// in place, and only the key "anthropic-beta" is filtered afterwards.
fn merged_headers(extra: Option<&[(String, String)]>) -> Headers {
    let mut headers = Headers::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    for (name, value) in extra.unwrap_or_default() {
        headers.insert(name.clone(), value.clone());
    }
    headers
}

fn beta_headers<'pair>(
    pairs: impl Iterator<Item = (&'pair String, &'pair String)>,
) -> BTreeMap<String, BetaHeader> {
    pairs
        .map(|(name, value)| (name.clone(), BetaHeader::Text(value.clone())))
        .collect()
}

fn messages_json(messages: &[Message]) -> Result<Json, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(messages)?)
}

// Absent or null is absent; any other JSON type is a parameter error that names the field.
fn pop_bool(params: &mut ParamMap, field: &'static str) -> Result<Option<bool>, ConverseCallError> {
    match params.shift_remove(field) {
        None | Some(ParamValue::Null) => Ok(None),
        Some(ParamValue::Bool(value)) => Ok(Some(value)),
        Some(_) => Err(ConverseCallError::Param { field }),
    }
}

fn pop_text(
    params: &mut ParamMap,
    field: &'static str,
) -> Result<Option<String>, ConverseCallError> {
    match params.shift_remove(field) {
        None | Some(ParamValue::Null) => Ok(None),
        Some(ParamValue::Text(value)) => Ok(Some(value)),
        Some(_) => Err(ConverseCallError::Param { field }),
    }
}
