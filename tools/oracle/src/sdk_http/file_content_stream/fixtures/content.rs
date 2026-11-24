use std::collections::BTreeMap;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::files::error::FilesError as AnthropicError;
use betterlite_anthropic::files::request::{
    FilesUrl, files_url, validate_environment as validate_anthropic_environment,
};
use betterlite_core::api::file_http_content::config::{
    BufferedResponse, ContentParams, FileContentConfig, Headers, QueryParams,
};
use betterlite_core::api::file_http_content::error::ContentError;
use betterlite_core::api::vector_store_http_write::error::BaseLlmException;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};

// AnthropicFilesConfig's content members: the files leaf's URL and auth over the mock, the body passed through
pub(crate) struct AnthropicContent<'settings> {
    pub(crate) settings: &'settings AnthropicSettings,
}

impl FileContentConfig for AnthropicContent<'_> {
    fn transform_content_request(
        &self,
        request: &FileContentRequest,
        params: &ContentParams,
    ) -> Result<(String, QueryParams), ContentError> {
        let url = files_url(
            params.api_base.as_deref(),
            self.settings,
            FilesUrl::Content(&request.file_id),
        )
        .map_err(provider_failure)?;
        Ok((url, QueryParams::new()))
    }

    fn validate_environment(
        &self,
        headers: Headers,
        params: &ContentParams,
    ) -> Result<Headers, ContentError> {
        let mut wire = http_headers(&headers)?;
        validate_anthropic_environment(
            &mut wire,
            params.api_key.as_ref(),
            None,
            params.api_base.as_deref(),
            self.settings,
        )
        .map_err(provider_failure)?;
        let mut validated = Headers::new();
        for (name, value) in &wire {
            let text = value.to_str().map_err(|_invalid| {
                exception(format!("the {} header is not text", name.as_str()))
            })?;
            validated.insert(name.as_str().to_owned(), text.to_owned());
        }
        Ok(validated)
    }

    // HttpxBinaryResponseContent(response=raw_response): the status, the headers and the bytes as they arrived
    fn transform_content_response(
        &self,
        response: &BufferedResponse,
        _logging: &mut LoggingState,
        _params: &ContentParams,
    ) -> Result<BinaryContent, ContentError> {
        Ok(BinaryContent {
            status: response.status_code,
            headers: response
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            body: response.body.clone(),
        })
    }

    fn error_class(&self, message: &str, status_code: u16, headers: Headers) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name, Json::Text(value)))
                .collect(),
            status_code_is_synthesized: false,
        }
    }
}

// a leaf failure keeps a provider status error's facts; a ValueError-class raise has no status of its own
fn provider_failure(error: AnthropicError) -> ContentError {
    ContentError::Provider(match error {
        AnthropicError::Provider(source) => {
            let headers = source
                .headers
                .unwrap_or_default()
                .into_iter()
                .map(|(name, value)| (name, Json::Text(value)))
                .collect();
            BaseLlmException {
                status_code: source.status_code,
                message: source.message,
                headers,
                status_code_is_synthesized: source.status_code_is_synthesized,
            }
        }
        other @ (AnthropicError::FileDataRequired
        | AnthropicError::MissingApiKey
        | AnthropicError::InvalidHeader { .. }
        | AnthropicError::InvalidBase { .. }
        | AnthropicError::Segment(_)
        | AnthropicError::Media(_)
        | AnthropicError::Decode { .. }
        | AnthropicError::Transport { .. }) => exception(other.to_string()),
    })
}

const fn exception(message: String) -> BaseLlmException {
    BaseLlmException {
        status_code: 0,
        message,
        headers: BTreeMap::new(),
        status_code_is_synthesized: false,
    }
}

// the caller's extra headers are a dict in LiteLLM and become the wire map here
fn http_headers(headers: &Headers) -> Result<HttpHeaders, ContentError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_invalid| {
            ContentError::Provider(exception(format!("invalid header {name}")))
        })?;
        let parsed = HeaderValue::from_str(value).map_err(|_invalid| {
            ContentError::Provider(exception(format!("invalid {header_name} value")))
        })?;
        map.insert(header_name, parsed);
    }
    Ok(map)
}
