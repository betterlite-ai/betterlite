use std::collections::BTreeMap;

use betterlite_anthropic::completion::{
    error::CompletionError,
    headers::validate_environment,
    prompt::PromptSource,
    request::transform_request,
    response::{ResponseInput, transform_response},
};
use betterlite_config::sections::anthropic_completion::OptionValues;
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::{ChatResponse, ChatUsage};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use betterlite_tokenizer::engine::Engine;
use chrono::{DateTime, Utc};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use super::super::completion::{CompletionCall, HandlerFuture};
use super::{ClientHandle, Failure};
use crate::operation::OperationError;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest, Upstream};
use crate::transform_retry::TransformRetry;

pub trait Transport {
    fn send<'call>(
        &'call self,
        request: &'call PreparedRequest,
    ) -> HandlerFuture<'call, Upstream, OperationError>;
}

impl Transport for ProxiedClient {
    fn send<'call>(
        &'call self,
        request: &'call PreparedRequest,
    ) -> HandlerFuture<'call, Upstream, OperationError> {
        Box::pin(request.send(self))
    }
}

pub struct Legacy<'deps> {
    pub prompts: &'deps PromptFactory,
    pub defaults: &'deps OptionValues,
    pub body_limit: usize,
}

pub type ChunkParser = fn(&IndexMap<String, Json>) -> Result<GenericChunk, CompletionError>;
pub enum Reply {
    Buffered(Box<ChatResponse>),
    Stream {
        response: Box<StreamResponse>,
        parser: ChunkParser,
    },
}

pub async fn complete<Client, ClientFailure, FileFailure>(
    call: &CompletionCall<ClientHandle<Client>, Engine>,
    client: &Client,
    settings: &Legacy<'_>,
    now: DateTime<Utc>,
) -> Result<Reply, Failure<ClientFailure, FileFailure>>
where
    Client: Transport,
    ClientFailure: std::error::Error + Send + Sync + 'static,
    FileFailure: std::error::Error + Send + Sync + 'static,
{
    let source = PromptSource {
        custom_prompt_dict: &BTreeMap::new(),
        prompts: settings.prompts,
        now,
    };
    let messages: Vec<Message> = serde_json::from_value(serde_json::to_value(&call.messages)?)?;
    let mut optional = call.optional_params.clone();
    let extra_body = match optional.shift_remove("extra_body") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(Failure::ExtraBody),
    };
    let mut body =
        transform_request(&source, &call.model, &messages, optional, settings.defaults).await?;
    body.extend(extra_body);
    if call.stream {
        body.insert("stream".to_owned(), Json::Bool(true));
    }
    let mut headers = HeaderMap::new();
    for (name, value) in &call.headers {
        let mut value = HeaderValue::try_from(value.expose_secret())?;
        value.set_sensitive(true);
        headers.insert(HeaderName::try_from(name.as_str())?, value);
    }
    let prepared = Box::new(PreparedChat {
        http: PreparedHttp {
            method: Method::POST,
            url: Url::parse(call.api_base.as_deref().ok_or(Failure::MissingBase)?)?,
            headers: validate_environment(headers, call.api_key.as_ref())?,
            body: serde_json::to_vec(&body)?,
            deadline: call.timeout,
            body_limit: Some(settings.body_limit),
        },
        retry: TransformRetry::DISABLED,
        client_model: None,
    });
    let prepared = if call.stream {
        PreparedRequest::Stream(prepared)
    } else {
        PreparedRequest::Chat(prepared)
    };
    let client = call
        .shared_session
        .as_ref()
        .map_or(client, |supplied| &supplied.transport);
    match client.send(&prepared).await? {
        Upstream::Streaming(response) => Ok(Reply::Stream {
            response: Box::new(response),
            parser: betterlite_anthropic::completion::stream::chunk_parser,
        }),
        Upstream::Buffered(response) => {
            let envelope = ChatResponse {
                id: call.model_response.id.clone(),
                object: call.model_response.object.clone(),
                created: u64::try_from(call.model_response.created).unwrap_or_default(),
                model: call.model_response.model.clone().unwrap_or_default(),
                choices: serde_json::from_value(serde_json::to_value(
                    &call.model_response.choices,
                )?)?,
                usage: ChatUsage::default(),
                system_fingerprint: call.model_response.system_fingerprint.clone(),
                service_tier: None,
            };
            let input = ResponseInput {
                model: &call.model,
                messages: &messages,
                status_code: response.status,
                body: &response.body,
            };
            Ok(Reply::Buffered(Box::new(
                transform_response(&source, &input, envelope, &call.encoding).await?,
            )))
        }
    }
}
