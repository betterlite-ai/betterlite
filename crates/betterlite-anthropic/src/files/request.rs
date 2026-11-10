use std::time::Duration;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::url_path::encode_segment;
use betterlite_prompts::media_metadata::{FileData, extract_file_data};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings, get_auth_header};

use super::error::FilesError;
use super::{DEFAULT_PURPOSE, FILES_BETA_HEADER, MESSAGE_BATCH_ID_PREFIX};

type Entry = Result<OutboundRequest, FilesError>;
#[derive(Debug, Clone)]
pub struct FilesClient {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub settings: AnthropicSettings,
    pub timeout: Duration,
}

pub struct Upload<'call> {
    pub file: Option<&'call FileData>,
    pub purpose: Option<&'call str>,
    pub boundary: String,
    pub headers: &'call HeaderMap,
}
pub struct FileCall<'call> {
    pub file_id: &'call str,
    pub headers: &'call HeaderMap,
}

pub struct ListCall<'call> {
    pub purpose: Option<&'call str>,
    pub headers: &'call HeaderMap,
}

#[derive(Clone, Copy)]
pub enum FilesUrl<'url> {
    Files,
    File(&'url str),
    Content(&'url str),
}

pub fn files_url(
    api_base: Option<&str>,
    settings: &AnthropicSettings,
    target: FilesUrl<'_>,
) -> Result<String, FilesError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(&settings.api_base);
    let trimmed = base.trim_end_matches('/');
    match target {
        FilesUrl::Files => Ok(format!("{trimmed}/v1/files")),
        FilesUrl::File(file_id) => {
            let encoded = encode_segment(file_id, "file_id")?;
            Ok(format!("{trimmed}/v1/files/{encoded}"))
        }
        FilesUrl::Content(file_id) => {
            let encoded = encode_segment(file_id, "file_id")?;
            if file_id.starts_with(MESSAGE_BATCH_ID_PREFIX) {
                Ok(format!("{trimmed}/v1/messages/batches/{encoded}/results"))
            } else {
                Ok(format!("{trimmed}/v1/files/{encoded}/content"))
            }
        }
    }
}

pub fn validate_environment(
    headers: &mut HeaderMap,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    params_api_base: Option<&str>,
    settings: &AnthropicSettings,
) -> Result<(), FilesError> {
    let resolved_base = api_base.or(params_api_base);
    let auth = get_auth_header(api_key, resolved_base, false, settings)
        .ok_or(FilesError::MissingApiKey)?;
    let mut value = HeaderValue::from_str(auth.value.expose_secret()).map_err(|_invalid| {
        FilesError::InvalidHeader {
            name: auth.name.to_owned(),
        }
    })?;
    value.set_sensitive(true);
    headers.insert(HeaderName::from_static(auth.name), value);
    headers.insert(
        HeaderName::from_static("anthropic-version"),
        HeaderValue::from_static(ANTHROPIC_VERSION),
    );
    headers.insert(
        HeaderName::from_static("anthropic-beta"),
        HeaderValue::from_static(FILES_BETA_HEADER),
    );
    Ok(())
}

pub fn create_file_parts(
    file: Option<&FileData>,
    purpose: Option<&str>,
    now_epoch: i64,
) -> Result<Vec<MultipartPart>, FilesError> {
    let Some(supplied) = file else {
        return Err(FilesError::FileDataRequired);
    };
    let extracted = extract_file_data(supplied)?;
    let filename = extracted
        .filename
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("file_{now_epoch}"));
    let sent_purpose = purpose.unwrap_or(DEFAULT_PURPOSE);
    Ok(vec![
        MultipartPart {
            name: "file".to_owned(),
            file_name: Some(filename),
            content_type: Some(extracted.content_type),
            data: extracted.content,
        },
        MultipartPart {
            name: "purpose".to_owned(),
            file_name: None,
            content_type: None,
            data: sent_purpose.as_bytes().to_vec(),
        },
    ])
}

impl FilesClient {
    pub fn create_request(&self, call: &Upload<'_>, now_epoch: i64) -> Entry {
        let parts = create_file_parts(call.file, call.purpose, now_epoch)?;
        let body = RequestBody::Multipart {
            boundary: call.boundary.clone(),
            parts,
        };
        self.request(Method::POST, FilesUrl::Files, call.headers, body)
    }

    pub fn file_request(&self, method: Method, call: &FileCall<'_>) -> Entry {
        self.request(
            method,
            FilesUrl::File(call.file_id),
            call.headers,
            RequestBody::Empty,
        )
    }

    pub fn list_request(&self, call: &ListCall<'_>) -> Entry {
        let mut request = self.request(
            Method::GET,
            FilesUrl::Files,
            call.headers,
            RequestBody::Empty,
        )?;
        request.query = purpose_query(call.purpose);
        Ok(request)
    }

    pub fn content_request(&self, call: &FileCall<'_>) -> Entry {
        self.request(
            Method::GET,
            FilesUrl::Content(call.file_id),
            call.headers,
            RequestBody::Empty,
        )
    }

    fn request(
        &self,
        method: Method,
        target: FilesUrl<'_>,
        caller_headers: &HeaderMap,
        body: RequestBody,
    ) -> Entry {
        let url = files_url(self.api_base.as_deref(), &self.settings, target)?;
        let mut headers = caller_headers.clone();
        validate_environment(
            &mut headers,
            self.api_key.as_ref(),
            None,
            self.api_base.as_deref(),
            &self.settings,
        )?;
        let parsed = Url::parse(&url).map_err(|source| FilesError::InvalidBase { source })?;
        Ok(OutboundRequest {
            method,
            url: parsed,
            query: Vec::new(),
            headers,
            body,
            deadline: self.timeout,
        })
    }
}

#[must_use]
pub fn purpose_query(purpose: Option<&str>) -> Vec<(String, String)> {
    purpose
        .filter(|value| !value.is_empty())
        .map(|value| vec![("purpose".to_owned(), value.to_owned())])
        .unwrap_or_default()
}
