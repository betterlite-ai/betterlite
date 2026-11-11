use std::collections::VecDeque;
use std::str;
use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use futures_util::StreamExt as _;
use http::HeaderMap;
use url::Url;

use crate::agentcore_response::error::BedrockFailure;
use crate::agentcore_response::json::parse_json_reply;
use crate::agentcore_stream::transport::{Chunks, HandlerKind, PostRequest, StreamHandler};
use crate::agentcore_stream::{
    AgentCoreStream, AgentCoreStreamError, FIRST_RESPONSE, PROVIDER, StreamDeps, StreamPostCall,
    StreamRequest,
};
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, ErrorReply, ResponseBody};

pub(crate) fn handler_for(deps: &StreamDeps<'_>, kind: HandlerKind) -> Arc<dyn StreamHandler> {
    match &deps.supplied {
        Some(supplied) if supplied.kind() == kind => Arc::clone(supplied),
        Some(_) | None => Arc::clone(&deps.default),
    }
}

pub(crate) fn provider_error(
    status: u16,
    message: String,
    headers: &HeaderMap,
    url: Option<&str>,
) -> AgentCoreStreamError {
    let pairs = header_pairs(headers);
    let reply = url.map(|endpoint| ErrorReply {
        response: ErrorResponse {
            status_code: status,
            request: ErrorRequest {
                method: "POST".to_owned(),
                url: endpoint.to_owned(),
            },
        },
        headers: pairs.clone(),
        body: ResponseBody::Read(String::new()),
    });
    let input = ErrorInput {
        status_code: status,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(pairs)),
        response: reply,
    };
    match BedrockError::try_from(input) {
        Ok(error) => AgentCoreStreamError::Provider {
            status,
            cause: BedrockFailure {
                cause: Box::new(error),
            },
        },
        Err(source) => AgentCoreStreamError::Header(source),
    }
}

#[must_use]
pub fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let text = value.to_str().map_or_else(
                |_| format!("{} bytes that are not text", value.as_bytes().len()),
                str::to_owned,
            );
            (name.as_str().to_owned(), text)
        })
        .collect()
}

pub const MAX_BODY_BYTES: usize = 0x0100_0000;

pub async fn open(
    request: &StreamRequest<'_>,
    deps: &StreamDeps<'_>,
    kind: HandlerKind,
) -> Result<AgentCoreStream, AgentCoreStreamError> {
    let handler = handler_for(deps, kind);
    let url = Url::parse(request.api_base).map_err(AgentCoreStreamError::Url)?;
    let body = request_body(request)?;
    let head = handler
        .post(PostRequest {
            url,
            headers: request.headers.clone(),
            body,
            stream: true,
            deadline: TransportLimits::DEFAULT.read_timeout,
        })
        .await
        .map_err(AgentCoreStreamError::Transport)?;
    let mut chunks = head.chunks;
    check_status(&head.headers, head.status, request.api_base, &mut chunks).await?;
    deps.logger.post_call(StreamPostCall {
        input: request.messages,
        api_key: "",
        original_response: FIRST_RESPONSE,
        complete_input_dict: request.data,
    });
    if is_json(&head.headers) {
        let content = json_reply(head.status, &head.headers, &mut chunks).await?;
        return Ok(AgentCoreStream::ready(
            request.model,
            Arc::clone(&deps.ids),
            &content,
        ));
    }
    Ok(AgentCoreStream {
        provider: PROVIDER,
        model: request.model.to_owned(),
        ids: Arc::clone(&deps.ids),
        chunks: Some(chunks),
        buffer: Vec::new(),
        pending: VecDeque::new(),
        failure: None,
    })
}

fn request_body(request: &StreamRequest<'_>) -> Result<Vec<u8>, AgentCoreStreamError> {
    match request.signed_json_body {
        Some(bytes) if !bytes.is_empty() => Ok(bytes.to_vec()),
        Some(_) | None => serde_json::to_vec(request.data).map_err(AgentCoreStreamError::Encode),
    }
}

async fn check_status(
    headers: &HeaderMap,
    status: u16,
    api_base: &str,
    chunks: &mut Chunks,
) -> Result<(), AgentCoreStreamError> {
    if status == 200 {
        return Ok(());
    }
    let error_body = read_body(chunks).await?;
    let message = String::from_utf8(error_body)
        .unwrap_or_else(|error| format!("{} bytes that are not text", error.as_bytes().len()));
    Err(provider_error(status, message, headers, Some(api_base)))
}

async fn json_reply(
    status: u16,
    headers: &HeaderMap,
    chunks: &mut Chunks,
) -> Result<String, AgentCoreStreamError> {
    let body = match read_body(chunks).await {
        Ok(body) => body,
        Err(error) => {
            return Err(provider_error(
                status,
                format!("AgentCore: Failed to read/parse JSON response body: {error}"),
                headers,
                None,
            ));
        }
    };
    let value: Json = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(source) => {
            return Err(provider_error(
                status,
                format!("AgentCore: Failed to read/parse JSON response body: {source}"),
                headers,
                None,
            ));
        }
    };
    let reply = parse_json_reply(&value).map_err(AgentCoreStreamError::Reply)?;
    Ok(reply.content)
}

async fn read_body(chunks: &mut Chunks) -> Result<Vec<u8>, AgentCoreStreamError> {
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let bytes = chunk.map_err(AgentCoreStreamError::Body)?;
        if body
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > MAX_BODY_BYTES)
        {
            return Err(AgentCoreStreamError::Body(HttpError::TooLarge {
                limit: MAX_BODY_BYTES,
            }));
        }
        body.extend_from_slice(&bytes);
    }
    Ok(body)
}

fn is_json(headers: &HeaderMap) -> bool {
    let values: Vec<&str> = headers
        .get_all("content-type")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    !values.is_empty()
        && values
            .join(", ")
            .to_lowercase()
            .contains("application/json")
}
