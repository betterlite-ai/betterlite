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
use serde_json::Value;

use crate::sagemaker_stream::events::EventState;
use crate::sagemaker_stream::{ErrorPolicy, SagemakerEvents};
use crate::signing::json_post::{JsonPost, sign_aws_json_post};

use super::convert;
use super::endpoint::{self, EndpointRequest, ParamMap};
use super::error::RuntimeError;
use super::runtime::{
    ChunkStream, PreparedCall, SagemakerCredentials, SagemakerRuntime, SagemakerTransport,
    epoch_seconds, stream_error, unreadable_reply, wire_error,
};
use super::{CONTENT_TYPE, JSON_CONTENT_TYPE};

const SERVICE: &str = "sagemaker";
const COMPONENT_HEADER: &str = "X-Amzn-SageMaker-Inference-Component";
const BASE_URL_FIELD: &str = "sagemaker_base_url";
const MODEL_ID_FIELD: &str = "model_id";

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

impl<T, C> SagemakerRuntime<'_, T, C>
where
    C: SagemakerCredentials,
{
    // _load_credentials, transform_request and _prepare_request, in the order the handler runs them
    pub async fn prepare_completion(
        &self,
        call: &CompletionCall<'_>,
    ) -> Result<PreparedCall, RuntimeError> {
        let region = endpoint::call_region(call.signing, self.environment.inputs())?;
        let credentials = self.credentials_for(call.signing, &region).await?;
        // the handler reads the base url, the component and the stream flag off the same options the body carries
        let base_url = text_option(call.optional_params, BASE_URL_FIELD)?;
        let model_id = text_option(call.optional_params, MODEL_ID_FIELD)?;
        let stream = matches!(call.optional_params.get("stream"), Some(Value::Bool(true)));
        let body = completion_body(call, DateTime::<Utc>::from(self.clock.now())).await?;
        let body_json = convert::completion_body(&body);
        let text = dumps_object(&body_json);
        let url = endpoint::runtime_url(&EndpointRequest {
            model: call.model,
            region: &region,
            base_url,
            stream,
        });
        let signed = sign_aws_json_post(JsonPost {
            service_name: SERVICE,
            region: Some(&region),
            url: &url,
            body: &text,
            headers: &[(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned())],
            credentials: &credentials,
            now: self.clock.now(),
        })?;
        // the component header is added after signing, so it stays outside SignedHeaders as it does there
        let mut headers = signed.headers;
        if let Some(model_id) = model_id {
            headers.retain(|(name, _)| !name.eq_ignore_ascii_case(COMPONENT_HEADER));
            headers.push((COMPONENT_HEADER.to_owned(), model_id.to_owned()));
        }
        Ok(PreparedCall {
            url: signed.url,
            headers,
            body: signed.body,
            body_json,
            region,
        })
    }
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    T: SagemakerTransport,
    C: SagemakerCredentials,
{
    pub async fn completion(
        &self,
        call: &CompletionCall<'_>,
    ) -> Result<ChatResponse, SagemakerError> {
        let prepared = self.prepare_completion(call).await?;
        let response = self.transport.send(self.outbound(&prepared)?).await?;
        if response.status != 200 {
            return Err(wire_error(response.status, &response.body));
        }
        let reply: Value = serde_json::from_slice(&response.body)
            .map_err(|source| unreadable_reply(&response.body, &source))?;
        let prompt = inputs(&prepared.body_json);
        let input = ReplyInput {
            reply: &reply,
            model: call.model,
            prompt: &prompt,
            id: format!("chatcmpl-{}", (self.identifier)()),
            created: epoch_seconds(self.clock.now()),
        };
        Ok(transform_response(&input, self.counter)?)
    }

    pub async fn completion_stream(
        &self,
        call: &CompletionCall<'_>,
    ) -> Result<SagemakerEvents<ChunkStream>, SagemakerError> {
        let prepared = self.prepare_completion(call).await?;
        let (status, chunks) = self
            .transport
            .open_stream(self.outbound(&prepared)?)
            .await?;
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
}

async fn completion_body(
    call: &CompletionCall<'_>,
    now: DateTime<Utc>,
) -> Result<IndexMap<String, Value>, RuntimeError> {
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

// a null entry is the absent one, as dict.get reads it; any other non-string names the field
fn text_option<'params>(
    params: &'params Options,
    name: &'static str,
) -> Result<Option<&'params str>, RuntimeError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_other) => Err(RuntimeError::Field { field: name }),
    }
}

fn inputs(body: &IndexMap<String, Plain>) -> String {
    match body.get("inputs") {
        Some(Plain::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

mod tests;
