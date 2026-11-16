use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::transport::{
    AsyncClient, ContainerHttpReply, ContainerHttpRequest, ContainerMethod, TransferError,
    UploadedFile,
};

// B173's pooled client with the deadline it was built to enforce; no call can change it
#[derive(Debug, Clone)]
pub struct ProxiedContainerClient {
    pub client: ProxiedClient,
    pub deadline: Duration,
}

fn outbound_headers(headers: &BTreeMap<String, String>) -> Result<HeaderMap, TransferError> {
    let mut converted = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::try_from(name.as_str()).map_err(|source| TransferError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value = HeaderValue::try_from(value.as_str()).map_err(|source| {
            TransportError::InvalidHeader {
                name: name.clone(),
                source,
            }
        })?;
        converted.append(header_name, header_value);
    }
    Ok(converted)
}

// the boundary is the file's own digest, so it never repeats inside the data and needs no random source
fn multipart_body(file: &UploadedFile) -> RequestBody {
    RequestBody::Multipart {
        boundary: format!("betterlite-{}", hex::encode(Sha256::digest(&file.content))),
        parts: vec![MultipartPart {
            name: "file".to_owned(),
            file_name: Some(file.filename.clone()),
            content_type: Some(file.content_type.clone()),
            data: file.content.clone(),
        }],
    }
}

pub fn outbound_request(
    request: &ContainerHttpRequest,
    deadline: Duration,
) -> Result<OutboundRequest, TransferError> {
    let url = Url::parse(&request.url).map_err(|source| TransferError::Url {
        url: request.url.clone(),
        source,
    })?;
    let method = match request.method {
        ContainerMethod::Get => Method::GET,
        ContainerMethod::Delete => Method::DELETE,
        ContainerMethod::Post => Method::POST,
    };
    Ok(OutboundRequest {
        method,
        url,
        query: request.query.clone(),
        headers: outbound_headers(&request.headers)?,
        body: request
            .file
            .as_ref()
            .map_or(RequestBody::Empty, multipart_body),
        deadline,
    })
}

// httpx reads a header value as UTF-8 and falls back to Latin-1, which maps every byte to a character
fn header_text(bytes: &[u8]) -> String {
    str::from_utf8(bytes).map_or_else(
        |_| bytes.iter().map(|byte| char::from(*byte)).collect(),
        str::to_owned,
    )
}

// httpx joins repeated headers with a comma and keeps names in lower case
#[must_use]
pub fn reply_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut joined: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let text = header_text(value.as_bytes());
        match joined.entry(name.as_str().to_owned()) {
            Entry::Occupied(mut present) => {
                present.get_mut().push_str(", ");
                present.get_mut().push_str(&text);
            }
            Entry::Vacant(slot) => {
                slot.insert(text);
            }
        }
    }
    joined
}

impl AsyncClient for ProxiedContainerClient {
    async fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> Result<ContainerHttpReply, TransferError> {
        let outbound = outbound_request(request, self.deadline)?;
        let response = self
            .client
            .send_following(outbound)
            .await?
            .buffered(self.client.limits.body_bytes)
            .await?;
        Ok(ContainerHttpReply {
            status: response.status,
            headers: reply_headers(&response.headers),
            body: response.body,
        })
    }
}
