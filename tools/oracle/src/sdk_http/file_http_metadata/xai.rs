use betterlite_config::sections::xai_identity::Settings;
use betterlite_core::api::file_http_metadata::config::{
    EnvironmentRequest, FileParams, FileRequest, FileResponse, Headers, ListRequest,
    MetadataConfig, NextPageRequest, PreparedRequest, ReplyContext,
};
use betterlite_core::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::{
    api::batch_records::WireDecodeLimits,
    files::records::{FileDeleted, FileObject},
    xai_batches::error::XaiBatchesError,
    xai_files::{
        FilesTarget, XaiFile, XaiFileDeleted, XaiFileList, decode_file_reply,
        decode_xai_upload_reply, error::XaiFilesError, files_url, next_page, to_file_deleted,
        to_file_list, to_file_object,
    },
    xai_identity::{KeyOrder, get_api_key},
};
use chrono::Utc;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

// LiteLLM's ValueError carries no status of its own and its BaseLLMException carries one
const MISSING_KEY: &str = "Missing xAI API Key. Pass api_key, set litellm.xai_key or XAI_API_KEY";
// The limits the files reply decode holds xAI answers to; the metadata bodies are far below them
const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 1 << 20,
    max_event_bytes: 1 << 10,
    max_items: 1000,
};

// XAIFilesConfig over B1015's typed operations: the config a provider task hands the handler
pub(super) struct XaiFilesConfig<'settings> {
    pub(super) settings: &'settings Settings,
}

impl XaiFilesConfig<'_> {
    fn endpoint(
        &self,
        params: &FileParams,
        target: FilesTarget<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        let api_base = params.get("api_base").and_then(|value| value.as_str());
        let url = files_url(api_base, target, self.settings).map_err(failure)?;
        Ok((url, FileParams::new()))
    }

    fn file_reply(&self, raw_response: &FileResponse) -> Result<FileObject, BaseLlmException> {
        decode_file_reply(&reply(raw_response), &LIMITS, |file: XaiFile| {
            to_file_object(&file, Utc::now().timestamp())
        })
        .map_err(failure)
    }

    fn deleted_reply(&self, raw_response: &FileResponse) -> Result<FileDeleted, BaseLlmException> {
        decode_file_reply(&reply(raw_response), &LIMITS, |file: XaiFileDeleted| {
            to_file_deleted(&file)
        })
        .map_err(failure)
    }

    fn listed_reply(
        &self,
        raw_response: &FileResponse,
    ) -> Result<Vec<FileObject>, BaseLlmException> {
        decode_file_reply(&reply(raw_response), &LIMITS, |page: XaiFileList| {
            to_file_list(&page, Utc::now().timestamp())
        })
        .map_err(failure)
    }
}

// httpx.Response carries http's header map; the contract flattens it, so a value that cannot come back is dropped
fn reply(response: &FileResponse) -> UpstreamResponse {
    let headers = response
        .headers
        .iter()
        .filter_map(|(name, value)| {
            let name = HeaderName::try_from(name.as_str()).ok()?;
            let value = HeaderValue::from_bytes(value.as_bytes()).ok()?;
            Some((name, value))
        })
        .collect();
    UpstreamResponse {
        status: response.status_code,
        headers,
        body: response.body.clone(),
    }
}

// raise_for_xai_status keeps the reply's text, status and headers; every other failure is the handler's
fn failure(error: XaiFilesError) -> BaseLlmException {
    match error {
        XaiFilesError::Shared(XaiBatchesError::Status {
            status,
            message,
            headers,
        }) => BaseLlmException {
            status_code: status,
            message,
            headers: headers
                .into_iter()
                .map(|(name, value)| (name, Json::Text(value)))
                .collect(),
            status_code_is_synthesized: false,
        },
        other => {
            let carried = other.status_code();
            BaseLlmException {
                status_code: carried.unwrap_or(500),
                message: other.to_string(),
                headers: HeaderMap::new(),
                status_code_is_synthesized: carried.is_none(),
            }
        }
    }
}

impl MetadataConfig for XaiFilesConfig<'_> {
    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, BaseLlmException> {
        let Some(key) = get_api_key(request.api_key, KeyOrder::Chat, self.settings) else {
            return Err(BaseLlmException {
                status_code: 401,
                message: MISSING_KEY.to_owned(),
                headers: HeaderMap::new(),
                status_code_is_synthesized: false,
            });
        };
        let mut headers = request.headers;
        headers.insert(
            AUTHORIZATION.as_str().to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        Ok(headers)
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.endpoint(request.litellm_params, FilesTarget::File(request.file_id))
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_response(
        &self,
        raw_response: &FileResponse,
        _context: ReplyContext<'_>,
    ) -> Result<FileObject, BaseLlmException> {
        self.file_reply(raw_response)
    }

    fn transform_delete_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.endpoint(request.litellm_params, FilesTarget::File(request.file_id))
    }

    fn transform_delete_file_response(
        &self,
        raw_response: &FileResponse,
        _context: ReplyContext<'_>,
    ) -> Result<FileDeleted, BaseLlmException> {
        self.deleted_reply(raw_response)
    }

    fn transform_list_files_request(
        &self,
        request: ListRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException> {
        self.endpoint(request.litellm_params, FilesTarget::Collection)
    }

    // ast-grep-ignore: no-long-names
    fn transform_list_files_next_request(
        &self,
        raw_response: &FileResponse,
        _optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Option<NextPageRequest> {
        let page: XaiFileList = decode_xai_upload_reply(&raw_response.body, &LIMITS).ok()?;
        let base = litellm_params
            .get("api_base")
            .and_then(|value| value.as_str());
        let url = files_url(base, FilesTarget::Collection, self.settings).ok()?;
        next_page(&page, &url).map(|next| (next.url, next.query.into_iter().collect()))
    }

    fn transform_list_files_response(
        &self,
        raw_response: &FileResponse,
        _context: ReplyContext<'_>,
    ) -> Result<Vec<FileObject>, BaseLlmException> {
        self.listed_reply(raw_response)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}
