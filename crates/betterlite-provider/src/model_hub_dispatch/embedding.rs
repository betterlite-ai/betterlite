use std::str::from_utf8;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::huggingface_chat::error::HuggingfaceError;
use betterlite_openai::huggingface_embedding::error::HuggingfaceEmbeddingError;
use betterlite_openai::huggingface_embedding::request::Params;
use betterlite_openai::huggingface_embedding::response::{
    EmbeddingResponse, OrdinaryEncoder, decode_embedding_response, process_embedding_response,
};
use betterlite_openai::huggingface_embedding::task::PipelineTagSource;
use betterlite_openai::huggingface_embedding::{PrepareRequest, PreparedRequest, prepare};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use http::Method;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};

use super::credentials::{HUGGINGFACE_EMBEDDING_KEY, Sources, huggingface_env, resolve};
use super::hub::{PrefetchedTag, headers_from, model_info};
use super::json::{self, additional_args, header_values};
use super::transport::WireRequest;
use super::{DispatchDeps, HubDispatchError};

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

// handler.py reads the model's pipeline_tag only when input_type is absent
async fn pipeline_tag(
    call: &EmbeddingCall<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<Box<dyn PipelineTagSource>, HubDispatchError> {
    if !matches!(
        call.optional_params.get("input_type"),
        None | Some(PromptJson::Null)
    ) {
        return Ok(Box::new(NoTag));
    }
    let info = model_info(deps, call.model, &Headers::new(), &[]).await?;
    Ok(Box::new(PrefetchedTag(info)))
}

async fn prepare_call(
    call: &EmbeddingCall<'_>,
    deps: &DispatchDeps<'_>,
) -> Result<PreparedCall, HubDispatchError> {
    let sources = Sources {
        call: call.api_key.map(ExposeSecret::expose_secret),
        globals: deps.globals,
        env: deps.environment,
    };
    let key = resolve(
        HUGGINGFACE_EMBEDDING_KEY,
        &sources,
        &mut PromptObject::new(),
    )?;
    let params: Params = json::convert(call.optional_params)?;
    let tag = pipeline_tag(call, deps).await?;
    let request = prepare(
        &PrepareRequest {
            model: call.model,
            input: call.input,
            optional_params: &params,
            headers: call.headers,
            api_key: key.as_ref(),
            api_base: call.api_base,
            env: &huggingface_env(deps.environment),
        },
        &*tag,
    )?;
    let data = body_object(&request.body)?;
    let input = Json::List(call.input.iter().cloned().map(Json::Text).collect());
    Ok(PreparedCall {
        request,
        data,
        input,
    })
}

// HTTPHandler.post raises on any non-2xx before the handler reads the body or logs post_call
fn status_failure(response: &UpstreamResponse) -> Result<(), HubDispatchError> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    let text =
        from_utf8(&response.body).map_err(|source| HubDispatchError::BodyNotUtf8 { source })?;
    Err(HubDispatchError::Embedding(
        HuggingfaceEmbeddingError::Huggingface(HuggingfaceError {
            status_code: response.status,
            message: text.to_owned(),
            headers: headers_from(&response.headers),
        }),
    ))
}

fn received(
    call: &EmbeddingCall<'_>,
    encoder: &dyn OrdinaryEncoder,
    log: &mut LogContext<'_, '_>,
    prepared: PreparedCall,
    response: &UpstreamResponse,
) -> Result<EmbeddingResponse, HubDispatchError> {
    status_failure(response)?;
    let logged = Some(RawBody::Object(prepared.data));
    let answer = PostCall {
        original_response: json::logged_reply(&response.body),
        input: prepared.input,
        additional_args: additional_args(None, None, logged, IndexMap::new()),
    };
    post_call(log.state, &answer, log.env);
    let reply = decode_embedding_response(&response.body)?;
    let embedding = process_embedding_response(&reply, call.model, call.input, encoder)?;
    Ok(embedding)
}

pub async fn embed<'call>(
    call: &'call EmbeddingCall<'_>,
    deps: &'call DispatchDeps<'_>,
    log: &'call mut LogContext<'_, '_>,
) -> Result<EmbeddingResponse, HubDispatchError> {
    let prepared = prepare_call(call, deps).await?;
    let sent = PreCall {
        input: prepared.input.clone(),
        model: None,
        additional_args: additional_args(
            Some(prepared.request.url.clone()),
            Some(header_values(&prepared.request.headers)),
            Some(RawBody::Object(prepared.data.clone())),
            IndexMap::new(),
        ),
    };
    pre_call(log.state, &sent, log.env).map_err(|source| HubDispatchError::PreCall { source })?;
    let request = WireRequest {
        method: Method::POST,
        url: &prepared.request.url,
        query: &[],
        headers: &prepared.request.headers,
        body: Some(prepared.request.body.as_bytes()),
        deadline: deps.timeout,
    };
    let response = deps.post.post(&request).await?;
    received(call, deps.encoder, log, prepared, &response)
}

pub struct NoTag;

impl PipelineTagSource for NoTag {
    fn tag(
        &self,
        _model: &str,
        _task_type: Option<&str>,
    ) -> Result<Option<String>, HuggingfaceError> {
        Ok(None)
    }
}
