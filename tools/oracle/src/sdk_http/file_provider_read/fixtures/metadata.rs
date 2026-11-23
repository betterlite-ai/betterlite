use std::collections::BTreeMap;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::files::error::FilesError as AnthropicError;
use betterlite_anthropic::files::request::{
    FilesUrl, files_url, purpose_query, validate_environment as validate_anthropic_environment,
};
use betterlite_anthropic::files::wire::{
    decode_anthropic_delete, decode_anthropic_file, decode_anthropic_file_list,
};
use betterlite_config::sections::anthropic_credentials::Settings as AnthropicCredentials;
use betterlite_core::api::file_http_metadata::config::{
    EnvironmentRequest, FileParams, FileRequest, FileResponse, Headers, ListRequest,
    MetadataConfig, NextPageRequest, PreparedRequest, ReplyContext,
};
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap as ExceptionHeaders,
};
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{FileDeleted, FileObject};
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

// AnthropicFilesConfig over the anthropic files leaf: the config a generic provider branch hands B2462's handler
pub(crate) struct AnthropicFiles<'settings> {
    pub(crate) settings: &'settings AnthropicSettings,
    pub(crate) clock: fn() -> i64,
}

impl AnthropicFiles<'_> {
    // transform_retrieve_file_request's URL, from litellm_params' api_base or the settings' default
    fn endpoint(
        &self,
        params: &FileParams,
        target: FilesUrl<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        let api_base = params.get("api_base").and_then(|value| value.as_str());
        let url = files_url(api_base, self.settings, target).map_err(failure)?;
        Ok((url, FileParams::new()))
    }
}

impl MetadataConfig for AnthropicFiles<'_> {
    // AnthropicModelInfo.get_auth_header, over the key litellm_params carries
    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, BaseLlmException> {
        let mut headers = http_headers(&request.headers)?;
        let params_base = request
            .litellm_params
            .get("api_base")
            .and_then(|value| value.as_str());
        validate_anthropic_environment(
            &mut headers,
            request.api_key,
            None,
            params_base,
            self.settings,
        )
        .map_err(failure)?;
        let mut validated = Headers::new();
        for (name, value) in &headers {
            let text = value.to_str().map_err(|_invalid| {
                exception(
                    0,
                    format!("the {} header is not text", name.as_str()),
                    BTreeMap::new(),
                )
            })?;
            validated.insert(name.as_str().to_owned(), text.to_owned());
        }
        Ok(validated)
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.endpoint(request.litellm_params, FilesUrl::File(request.file_id))
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_response(
        &self,
        raw_response: &FileResponse,
        _context: ReplyContext<'_>,
    ) -> Result<FileObject, BaseLlmException> {
        decode_anthropic_file(&raw_response.body, (self.clock)()).map_err(failure)
    }

    fn transform_delete_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.endpoint(request.litellm_params, FilesUrl::File(request.file_id))
    }

    fn transform_delete_file_response(
        &self,
        raw_response: &FileResponse,
        _context: ReplyContext<'_>,
    ) -> Result<FileDeleted, BaseLlmException> {
        decode_anthropic_delete(&raw_response.body).map_err(failure)
    }

    // the purpose query the leaf's list request puts on the URL, and nothing else
    fn transform_list_files_request(
        &self,
        request: ListRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        let (url, _) = self.endpoint(request.litellm_params, FilesUrl::Files)?;
        let query = purpose_query(request.purpose)
            .into_iter()
            .map(|(name, value)| (name, Value::String(value)))
            .collect();
        Ok((url, query))
    }

    // ast-grep-ignore: no-long-names
    fn transform_list_files_next_request(
        &self,
        _raw_response: &FileResponse,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Option<NextPageRequest> {
        None
    }

    fn transform_list_files_response(
        &self,
        raw_response: &FileResponse,
        _context: ReplyContext<'_>,
    ) -> Result<Vec<FileObject>, BaseLlmException> {
        decode_anthropic_file_list(&raw_response.body, (self.clock)()).map_err(failure)
    }

    fn error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: ExceptionHeaders,
    ) -> BaseLlmException {
        exception(status_code, message.to_owned(), headers)
    }
}

pub(crate) fn anthropic_settings(credentials: &AnthropicCredentials) -> AnthropicSettings {
    AnthropicSettings {
        api_key: credentials.api_key.clone(),
        auth_token: credentials.auth_token.clone(),
        api_base: credentials.api_base.clone(),
        use_bearer_for_custom_base: credentials.use_bearer_for_custom_base,
        anthropic_version: AnthropicSettings::default().anthropic_version,
    }
}

// litellm_params as get_litellm_params leaves it before the handler: the call's api_key and api_base, either of which may be None
pub(crate) fn metadata_params(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
) -> FileParams {
    let key = api_key.map_or(Value::Null, |key| {
        Value::String(key.expose_secret().to_owned())
    });
    let base = api_base.map_or(Value::Null, |base| Value::String(base.to_owned()));
    let mut params = FileParams::new();
    params.insert("api_key".to_owned(), key);
    params.insert("api_base".to_owned(), base);
    params
}

// a leaf failure keeps a provider status error's facts; a ValueError-class raise has no status of its own
fn failure(error: AnthropicError) -> BaseLlmException {
    match error {
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
        | AnthropicError::Transport { .. }) => exception(0, other.to_string(), BTreeMap::new()),
    }
}

const fn exception(
    status_code: u16,
    message: String,
    headers: ExceptionHeaders,
) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message,
        headers,
        status_code_is_synthesized: false,
    }
}

// the caller's extra headers are a dict in LiteLLM and become the wire map here
fn http_headers(headers: &Headers) -> Result<HttpHeaders, BaseLlmException> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_invalid| {
            exception(
                0,
                format!("the header name {name} is not valid"),
                BTreeMap::new(),
            )
        })?;
        let parsed = HeaderValue::from_str(value).map_err(|_invalid| {
            exception(
                0,
                format!("the {header_name} header value is not valid"),
                BTreeMap::new(),
            )
        })?;
        map.insert(header_name, parsed);
    }
    Ok(map)
}
