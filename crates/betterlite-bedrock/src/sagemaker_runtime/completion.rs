use std::collections::{BTreeMap, VecDeque};

use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json as Plain;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::sagemaker_completion::params::Options;
use betterlite_openai::sagemaker_completion::request::{
    CustomPromptEntry, RequestInput, transform_request,
};
use betterlite_openai::sagemaker_completion::response::{ReplyInput, transform_response};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use bytes::BytesMut;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use crate::credentials::CredentialSource;
use crate::sagemaker_stream::events::EventState;
use crate::sagemaker_stream::{ErrorPolicy, SagemakerEvents};
use crate::signing::json_post::{JsonPost, sign_aws_json_post};

use super::convert;
use super::endpoint::{self, EndpointRequest, ParamMap};
use super::error::{RuntimeError, RuntimeFailure, timeout_error};
use super::runtime::{
    ChunkStream, PreparedCall, SagemakerRuntime, SagemakerTransport, epoch_seconds, stream_error,
    wire_error,
};
use super::{CONTENT_TYPE, JSON_CONTENT_TYPE};

const COMPONENT_HEADER: &str = "X-Amzn-SageMaker-Inference-Component";

pub struct CompletionCall<'call> {
    pub model: &'call str,
    pub messages: &'call [Message],
    pub optional_params: &'call Options,
    pub signing: &'call ParamMap,
    pub custom_prompt_dict: &'call BTreeMap<String, CustomPromptEntry>,
    pub global_custom_prompt_dict: &'call BTreeMap<String, CustomPromptEntry>,
    pub hf_model_name: Option<&'call str>,
    pub prompts: &'call PromptFactory,
}

pub async fn prepare_completion<T, C>(
    runtime: &SagemakerRuntime<'_, T, C>,
    call: &CompletionCall<'_>,
) -> Result<PreparedCall, RuntimeError>
where
    T: Sync,
    C: CredentialSource,
{
    let region = endpoint::call_region(call.signing, runtime.environment.inputs())?;
    let credentials = runtime.credentials_for(call.signing, &region).await?;
    let base_url = text_option(call.optional_params, "sagemaker_base_url")?;
    let model_id = text_option(call.optional_params, "model_id")?;
    let stream = call
        .optional_params
        .get("stream")
        .is_some_and(|flag| flag.as_bool() == Some(true));
    let body = completion_body(call, DateTime::<Utc>::from(runtime.clock.now())).await?;
    let body_json = convert::completion_body(&body)?;
    let text = dumps_object(&body_json);
    let url = endpoint::runtime_url(&EndpointRequest {
        model: call.model,
        region: &region,
        base_url,
        stream,
    });
    let signed = sign_aws_json_post(JsonPost {
        service_name: "sagemaker",
        region: Some(&region),
        url: &url,
        body: &text,
        headers: &[(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned())],
        credentials: &credentials,
        now: runtime.clock.now(),
    })?;
    // the component header is added after signing, so it stays outside SignedHeaders as it does there
    let mut headers = signed.headers;
    if let Some(component) = model_id {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case(COMPONENT_HEADER));
        headers.push((COMPONENT_HEADER.to_owned(), component.to_owned()));
    }
    Ok(PreparedCall {
        url: signed.url,
        headers,
        body: signed.body,
        body_json,
        region,
    })
}

pub async fn completion<T, C>(
    runtime: &SagemakerRuntime<'_, T, C>,
    call: &CompletionCall<'_>,
) -> Result<ChatResponse, RuntimeFailure>
where
    T: SagemakerTransport,
    C: CredentialSource,
{
    let prepared = prepare_completion(runtime, call).await?;
    let response = runtime
        .transport
        .send(runtime.outbound(&prepared)?)
        .await
        .map_err(RuntimeError::from)?;
    if response.status != 200 {
        return Err(RuntimeFailure::from(wire_error(
            response.status,
            &response.body,
        )));
    }
    let reply = serde_json::from_slice(&response.body)?;
    let prompt = inputs(&prepared.body_json);
    let input = ReplyInput {
        reply: &reply,
        model: call.model,
        prompt: &prompt,
        id: format!("chatcmpl-{}", (runtime.identifier)()),
        created: epoch_seconds(runtime.clock.now()),
    };
    Ok(transform_response(&input, runtime.counter)?)
}

pub async fn completion_stream<T, C>(
    runtime: &SagemakerRuntime<'_, T, C>,
    call: &CompletionCall<'_>,
) -> Result<SagemakerEvents<ChunkStream>, SagemakerError>
where
    T: SagemakerTransport,
    C: CredentialSource,
{
    let prepared = prepare_completion(runtime, call).await?;
    let (status, chunks) = runtime
        .transport
        .open_stream(runtime.outbound(&prepared)?)
        .await
        .map_err(timeout_error)?;
    if status != 200 {
        return Err(stream_error(status, chunks).await);
    }
    Ok(SagemakerEvents {
        state: EventState {
            // AWSEventStreamDecoder(model="") leaves the chunks' model empty
            model: String::new(),
            is_messages_api: false,
            policy: ErrorPolicy::Skip,
            frames: BytesMut::new(),
            accumulated: String::new(),
        },
        chunks,
        pending: VecDeque::new(),
        done: false,
    })
}

async fn completion_body(
    call: &CompletionCall<'_>,
    now: DateTime<Utc>,
) -> Result<Options, RuntimeError> {
    transform_request(&RequestInput {
        model: call.model,
        messages: call.messages,
        optional_params: call.optional_params,
        custom_prompt_dict: call.custom_prompt_dict,
        global_custom_prompt_dict: call.global_custom_prompt_dict,
        hf_model_name: call.hf_model_name,
        prompts: call.prompts,
        now,
    })
    .await
    .map_err(RuntimeError::from)
}

fn text_option<'params>(
    params: &'params Options,
    name: &'static str,
) -> Result<Option<&'params str>, RuntimeError> {
    let Some(value) = params.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(Some)
        .ok_or(RuntimeError::Field { field: name })
}

fn inputs(body: &IndexMap<String, Plain>) -> String {
    match body.get("inputs") {
        Some(Plain::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

mod tests;
