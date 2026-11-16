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

fn body_object(body: &str) -> Result<BodyObject, HubDispatchError> {
    match serde_json::from_str::<Json>(body)
        .map_err(|source| HubDispatchError::Encode { source })?
    {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(HubDispatchError::BodyNotObject)
        }
    }
}

fn args(
    api_base: Option<String>,
    headers: Option<IndexMap<String, Json>>,
    body: Option<RawBody>,
) -> AdditionalArgs {
    AdditionalArgs {
        api_base,
        headers,
        complete_input_dict: body,
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

async fn prepare_call(
    call: &EmbeddingCall<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<PreparedCall, HubDispatchError> {
    let key = huggingface_embedding_key(call.api_key, deps.globals, deps.environment);
    let environment = huggingface_env(deps.environment);
    let params: Params = json::convert(call.optional_params)?;
    let prefetched = match call.optional_params.get("input_type") {
        None | Some(PromptJson::Null) => Some(PrefetchedTag(
            hub::fetch_pipeline_tag(deps.hub, call.model).await?,
        )),
        Some(_) => None,
    };
    let tag: &dyn PipelineTagSource = match prefetched.as_ref() {
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
        input: Json::List(
            call.input
                .iter()
                .map(|text| Json::Text(text.clone()))
                .collect(),
        ),
    })
}

pub async fn embed(
    call: &EmbeddingCall<'_>,
    deps: &DispatchDeps<'_>,
    log: &mut CallLogging<'_, '_>,
) -> Result<EmbeddingResponse, HubDispatchError> {
    let prepared = prepare_call(call, deps).await?;
    let headers: IndexMap<String, Json> = prepared
        .request
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    pre_call(
        log.state,
        &PreCall {
            input: prepared.input.clone(),
            model: None,
            additional_args: args(
                Some(prepared.request.url.clone()),
                Some(headers),
                Some(RawBody::Object(prepared.data.clone())),
            ),
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
            additional_args: args(None, None, Some(RawBody::Object(prepared.data))),
        },
        log.env,
    );
    let reply = decode_embedding_response(&response.body)?;
    let embedding = process_embedding_response(&reply, call.model, call.input, &deps.encoder)?;
    Ok(embedding)
}

struct NoTag;

impl PipelineTagSource for NoTag {
    fn tag(
        &self,
        _model: &str,
        _task_type: Option<&str>,
    ) -> Result<Option<String>, HuggingfaceError> {
        Ok(None)
    }
}
