use std::time::Duration;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::ExposeSecret as _;

use crate::files::FilesClient;
use crate::files::error::FilesError;
use crate::files::records::{FileContentRequest, FileCreate};

// the OpenAI SDK's path_template keeps RFC 3986 sub-delims, ':' and '@' inside a path segment
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b':')
    .remove(b'@');

const ORGANIZATION: HeaderName = HeaderName::from_static("openai-organization");

pub(super) struct Wire<'headers> {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub accept: &'static str,
    pub extra_headers: Option<&'headers IndexMap<String, String>>,
    pub timeout: Option<Duration>,
    pub body: RequestBody,
}

pub(super) fn file_path(file_id: &str, suffix: &str) -> Result<String, FilesError> {
    if file_id.is_empty() {
        return Err(FilesError::EmptyValue { field: "file_id" });
    }
    let path = format!("/files/{}{suffix}", utf8_percent_encode(file_id, SEGMENT));
    for segment in path.split('/') {
        // quoting never turns a dot into %2e, so only the literal dots can form a dot-segment
        if segment == "." || segment == ".." {
            return Err(FilesError::DotSegment {
                path: path.clone(),
                segment: segment.to_owned(),
            });
        }
    }
    Ok(path)
}

fn header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), FilesError> {
    let header_name =
        HeaderName::from_bytes(name.as_bytes()).map_err(|source| FilesError::Header {
            source: http::Error::from(source),
        })?;
    let header_value = HeaderValue::from_str(value).map_err(|source| FilesError::Header {
        source: http::Error::from(source),
    })?;
    Ok((header_name, header_value))
}

fn multipart_parts(data: &FileCreate) -> Vec<MultipartPart> {
    let mut fields = IndexMap::from([("purpose".to_owned(), data.purpose.wire_name().to_owned())]);
    if let Some(expires) = data.expires_after {
        fields.insert("expires_after[anchor]".to_owned(), "created_at".to_owned());
        fields.insert(
            "expires_after[seconds]".to_owned(),
            expires.seconds.to_string(),
        );
    }
    fields.extend(data.extra_body.clone());
    let mut parts: Vec<MultipartPart> = fields
        .into_iter()
        .map(|(name, value)| MultipartPart {
            name,
            file_name: None,
            content_type: None,
            data: value.into_bytes(),
        })
        .collect();
    parts.push(MultipartPart {
        name: "file".to_owned(),
        file_name: Some(
            data.file
                .filename
                .clone()
                .unwrap_or_else(|| "upload".to_owned()),
        ),
        content_type: Some(
            data.file
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
        ),
        data: data.file.data.clone(),
    });
    parts
}

impl FilesClient {
    pub(super) fn outbound(&self, wire: Wire<'_>) -> Result<OutboundRequest, FilesError> {
        let url = self
            .base_url
            .join(wire.path.trim_start_matches('/'))
            .map_err(|source| FilesError::BaseUrl { source })?;
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static(wire.accept));
        let bearer = format!("Bearer {}", self.api_key.expose_secret());
        let mut authorization = header_pair("authorization", &bearer)?.1;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        if let Some(organization) = &self.organization {
            headers.insert(
                ORGANIZATION,
                header_pair("openai-organization", organization)?.1,
            );
        }
        for (extra_name, extra_value) in wire.extra_headers.into_iter().flatten() {
            let (header_name, header_value) = header_pair(extra_name, extra_value)?;
            headers.insert(header_name, header_value);
        }
        Ok(OutboundRequest {
            method: wire.method,
            url,
            query: wire.query,
            headers,
            body: wire.body,
            deadline: wire.timeout.unwrap_or(self.timeout),
        })
    }

    pub fn create_request(
        &self,
        data: &FileCreate,
        boundary: String,
    ) -> Result<OutboundRequest, FilesError> {
        let parts = multipart_parts(data);
        self.outbound(Wire {
            method: Method::POST,
            path: "/files".to_owned(),
            query: Vec::new(),
            accept: "application/json",
            extra_headers: Some(&data.extra_headers),
            timeout: data.timeout,
            body: RequestBody::Multipart { boundary, parts },
        })
    }

    pub fn content_request(
        &self,
        request: &FileContentRequest,
    ) -> Result<OutboundRequest, FilesError> {
        self.outbound(Wire {
            method: Method::GET,
            path: file_path(&request.file_id, "/content")?,
            query: Vec::new(),
            accept: "application/binary",
            extra_headers: Some(&request.extra_headers),
            timeout: request.timeout,
            body: RequestBody::Empty,
        })
    }

    pub fn bare_request(
        &self,
        method: Method,
        path: String,
        query: Vec<(String, String)>,
    ) -> Result<OutboundRequest, FilesError> {
        self.outbound(Wire {
            method,
            path,
            query,
            accept: "application/json",
            extra_headers: None,
            timeout: None,
            body: RequestBody::Empty,
        })
    }
}
