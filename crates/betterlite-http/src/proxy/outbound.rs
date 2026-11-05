use std::time::Duration;

use reqwest::header::{
    AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, PROXY_AUTHORIZATION,
};
use reqwest::{Client, Method, Request, Url};

use crate::proxy::multipart::{MultipartPart, encode};
use crate::proxy::transport_error::TransportError;

const SECRET_HEADERS: [HeaderName; 4] = [
    AUTHORIZATION,
    PROXY_AUTHORIZATION,
    HeaderName::from_static("x-api-key"),
    HeaderName::from_static("api-key"),
];

#[derive(Debug)]
pub enum RequestBody {
    Empty,
    Json(Vec<u8>),
    Bytes {
        content_type: HeaderValue,
        data: Vec<u8>,
    },
    Multipart {
        boundary: String,
        parts: Vec<MultipartPart>,
    },
}

#[derive(Debug)]
pub struct OutboundRequest {
    pub method: Method,
    pub url: Url,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub body: RequestBody,
    pub deadline: Duration,
}

// A key in the URL beats the same key in the explicit params; repeats stay inside their own source.
pub fn merge_query(url: &mut Url, explicit: &[(String, String)]) {
    if explicit.is_empty() {
        return;
    }
    let mut merged: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let in_url = merged.len();
    for (key, value) in explicit {
        let collides = merged
            .iter()
            .take(in_url)
            .any(|(present, _)| present == key);
        if !collides {
            merged.push((key.clone(), value.clone()));
        }
    }
    url.query_pairs_mut().clear().extend_pairs(&merged);
}

impl OutboundRequest {
    pub(super) fn build(self, client: &Client) -> Result<Request, TransportError> {
        let mut url = self.url;
        merge_query(&mut url, &self.query);
        let mut headers = self.headers;
        let data = match self.body {
            RequestBody::Empty => Vec::new(),
            RequestBody::Json(data) => {
                headers
                    .entry(CONTENT_TYPE)
                    .or_insert(HeaderValue::from_static("application/json"));
                data
            }
            RequestBody::Bytes { content_type, data } => {
                headers.entry(CONTENT_TYPE).or_insert(content_type);
                data
            }
            RequestBody::Multipart { boundary, parts } => {
                let content_type = format!("multipart/form-data; boundary={boundary}");
                let value = HeaderValue::try_from(content_type).map_err(|_invalid| {
                    TransportError::MultipartName {
                        name: boundary.clone(),
                    }
                })?;
                headers.insert(CONTENT_TYPE, value);
                encode(&boundary, &parts)?
            }
        };
        for (name, value) in &mut headers {
            if SECRET_HEADERS.contains(name) {
                value.set_sensitive(true);
            }
        }
        client
            .request(self.method, url)
            .timeout(self.deadline)
            .headers(headers)
            .body(data)
            .build()
            .map_err(TransportError::Build)
    }
}

mod tests;
