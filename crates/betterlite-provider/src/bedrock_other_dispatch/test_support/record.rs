use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::{HeaderMap, HeaderName, HeaderValue};

// One posted request, in the shape every family's transport seam produced
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub deadline: Option<Duration>,
}

#[derive(Clone, Default)]
pub struct Seen(Arc<Mutex<Vec<Recorded>>>);

impl Seen {
    #[must_use]
    pub fn requests(&self) -> Vec<Recorded> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn push(&self, request: Recorded) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }
}

pub fn recorded(request: OutboundRequest) -> Recorded {
    let OutboundRequest {
        method,
        url,
        headers,
        body,
        deadline,
        ..
    } = request;
    let payload = match body {
        RequestBody::Json(bytes) => bytes,
        RequestBody::Bytes { data, .. } => data,
        RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
    };
    Recorded {
        method: method.as_str().to_owned(),
        url: url.to_string(),
        headers: headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect(),
        body: payload,
        deadline: Some(deadline),
    }
}

#[must_use]
pub fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let (Ok(header_name), Ok(header_value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) else {
            continue;
        };
        headers.insert(header_name, header_value);
    }
    headers
}
