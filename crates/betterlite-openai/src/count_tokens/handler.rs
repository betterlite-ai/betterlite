use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use secrecy::SecretString;
use serde_json::{Map, Value};
use url::Url;

use crate::count_tokens::error::CountTokensError;
use crate::count_tokens::request::{
    CountRequestBody, ResponsesInput, flatten_tools, required_headers, validate_request,
};

pub const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

#[derive(Debug)]
pub struct CountTokensHandler {
    pub client: ProxiedClient,
    pub request_timeout: Duration,
}

#[derive(Debug)]
pub struct HandlerRequest<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub api_key: &'call SecretString,
    pub api_base: Option<&'call str>,
    pub timeout: Option<Duration>,
    pub tools: Option<&'call [Map<String, Value>]>,
    pub instructions: Option<&'call str>,
}

// A JSON null answer is LiteLLM's "no result", so the counter returns None for it
pub fn decode_input_token_count(
    body: &[u8],
) -> Result<Option<Map<String, Value>>, CountTokensError> {
    match serde_json::from_slice(body).map_err(|source| CountTokensError::Decode { source })? {
        Value::Null => Ok(None),
        Value::Object(fields) => Ok(Some(fields)),
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(CountTokensError::NotAnObject)
        }
    }
}

impl CountTokensHandler {
    pub fn prepare(
        &self,
        request: &HandlerRequest<'_>,
    ) -> Result<OutboundRequest, CountTokensError> {
        validate_request(request.model, request.input)?;
        let tools = match request.tools {
            Some(tools) => Some(flatten_tools(tools)?),
            None => None,
        };
        let body = CountRequestBody {
            model: request.model,
            input: request.input,
            instructions: request.instructions,
            tools,
        };
        let base = request
            .api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(DEFAULT_API_BASE)
            .trim_end_matches('/');
        let url = Url::parse(&format!("{base}/responses/input_tokens"))
            .map_err(|source| CountTokensError::Endpoint { source })?;
        Ok(OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: required_headers(request.api_key)?,
            body: RequestBody::Json(
                serde_json::to_vec(&body).map_err(|source| CountTokensError::Encode { source })?,
            ),
            deadline: request.timeout.unwrap_or(self.request_timeout),
        })
    }

    async fn send(
        &self,
        request: &HandlerRequest<'_>,
    ) -> Result<Option<Map<String, Value>>, CountTokensError> {
        tracing::debug!(
            model = request.model,
            "Processing OpenAI CountTokens request"
        );
        let outbound = self.prepare(request)?;
        tracing::debug!(url = %outbound.url, "Making request");
        let response = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(|source| CountTokensError::Transport { source })?;
        tracing::debug!(status = response.status, "Response status");
        if response.status != 200 {
            let status = response.status;
            let text = String::from_utf8(response.body)
                .map_err(|_invalid| CountTokensError::BodyNotUtf8 { status })?;
            tracing::error!("OpenAI API error: {text}");
            return Err(CountTokensError::Status {
                status: response.status,
                text,
            });
        }
        decode_input_token_count(&response.body)
    }

    pub async fn handle_count_tokens_request(
        &self,
        request: &HandlerRequest<'_>,
    ) -> Result<Option<Map<String, Value>>, CountTokensError> {
        self.send(request)
            .await
            .map_err(CountTokensError::into_openai_error)
    }
}
