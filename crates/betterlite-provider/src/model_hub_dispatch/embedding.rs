use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::huggingface_chat::error::HuggingfaceError;
use betterlite_openai::huggingface_embedding::request::Params;
use betterlite_openai::huggingface_embedding::response::{
    EmbeddingResponse, decode_embedding_response, process_embedding_response,
};
use betterlite_openai::huggingface_embedding::task::PipelineTagSource;
use betterlite_openai::huggingface_embedding::{PrepareRequest, PreparedRequest, prepare};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::credentials::{huggingface_embedding_key, huggingface_env};
use super::hub::{self, PrefetchedTag};
use super::json;
use super::{CallLogging, DispatchDeps, HubDispatchError};

type BodyObject = IndexMap<String, Json>;

// main.py L6310's keyword parameters for one embedding call, as the huggingface arm at L6759 sees them
pub struct EmbeddingCall<'call> {
    pub model: &'call str,
    pub input: &'call [String],
    pub optional_params: &'call PromptObject,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub headers: &'call Headers,
}

struct PreparedCall {
    request: PreparedRequest,
    data: BodyObject,
    input: Json,
}

// embedding/handler.py L359: the pipeline tag is read only when the caller left input_type out
fn needs_tag(optional_params: &PromptObject) -> bool {
    matches!(
        optional_params.get("input_type"),
        None | Some(PromptJson::Null)
    )
}

fn logged_headers(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

fn input_json(input: &[String]) -> Json {
    Json::List(input.iter().map(|text| Json::Text(text.clone())).collect())
}

fn body_object(body: &str) -> Result<BodyObject, HubDispatchError> {
    match serde_json::from_str::<Json>(body).map_err(|source| HubDispatchError::Encode { source })? {
        Json::Object(fields) => Ok(fields),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_) => Err(HubDispatchError::BodyNotObject),
    }
}

async fn prepare_call(
    call: &EmbeddingCall<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<PreparedCall, HubDispatchError> {
    let key = huggingface_embedding_key(call.api_key, deps.globals, deps.environment);
    let environment = huggingface_env(deps.environment);
    let params: Params = json::convert(call.optional_params)?;
    let prefetched = if needs_tag(call.optional_params) {
        Some(PrefetchedTag(
            hub::fetch_pipeline_tag(deps.hub, call.model).await?,
        ))
    } else {
        None
    };
    let tag: &dyn PipelineTagSource = match &prefetched {
        Some(prefetched) => prefetched,
        None => &NoTag,
    };
    let request = prepare(
        &PrepareRequest {
            model: call.model,
            input: call.input,
            optional_params: &params,
            headers: call.headers,
            api_key: key.as_ref(),
            api_base: call.api_base,
            env: &environment,
        },
        tag,
    )?;
    let data = body_object(&request.body)?;
    Ok(PreparedCall {
        request,
        data,
        input: input_json(call.input),
    })
}

fn pre_call_args(prepared: &PreparedCall) -> AdditionalArgs {
    AdditionalArgs {
        api_base: Some(prepared.request.url.clone()),
        headers: Some(logged_headers(&prepared.request.headers)),
        complete_input_dict: Some(RawBody::Object(prepared.data.clone())),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

fn post_call_args(prepared: &PreparedCall) -> AdditionalArgs {
    AdditionalArgs {
        api_base: None,
        headers: None,
        complete_input_dict: Some(RawBody::Object(prepared.data.clone())),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

// handler.py L362-L391: log pre_call, POST the body, log post_call, then read the reply without a status check
pub async fn embed(
    call: &EmbeddingCall<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
) -> Result<EmbeddingResponse, HubDispatchError> {
    let prepared = prepare_call(call, deps).await?;
    pre_call(
        log.state,
        &PreCall {
            input: prepared.input.clone(),
            model: None,
            additional_args: pre_call_args(&prepared),
        },
        log.env,
    )
    .map_err(|source| HubDispatchError::PreCall { source })?;
    let response = deps
        .post
        .post(
            &prepared.request.url,
            &prepared.request.headers,
            prepared.request.body.as_bytes(),
            deps.timeout,
        )
        .await?;
    post_call(
        log.state,
        &PostCall {
            original_response: json::logged_reply(&response.body),
            input: prepared.input,
            additional_args: post_call_args(&prepared),
        },
        log.env,
    );
    let reply = decode_embedding_response(&response.body)?;
    Ok(process_embedding_response(
        &reply,
        call.model,
        call.input,
        &deps.encoder,
    )?)
}

// the branch where input_type already names the task, so no pipeline tag GET happens
struct NoTag;

impl PipelineTagSource for NoTag {
    fn tag(&self, _model: &str, _task_type: Option<&str>) -> Result<Option<String>, HuggingfaceError> {
        Ok(None)
    }
}
