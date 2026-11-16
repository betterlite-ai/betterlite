use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::fal::video_results::transport::VideoTransport;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::HeaderMap;
use url::Url;

// the transport records what the retrieval code sent and answers from the case's single scripted reply
pub(crate) struct ScriptedTransport {
    reply: Option<(u16, Vec<u8>)>,
    requests: Mutex<Vec<(String, BTreeMap<String, String>)>>,
}

impl ScriptedTransport {
    pub(crate) fn new(status: Option<u16>, body: &[u8]) -> Self {
        Self {
            reply: status.map(|status| (status, body.to_vec())),
            requests: Mutex::default(),
        }
    }

    pub(crate) fn requests(&self) -> Vec<(String, BTreeMap<String, String>)> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, url: &Url, headers: &HeaderMap) {
        let values = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.as_str().to_owned(), values));
    }
}

impl VideoTransport for ScriptedTransport {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        self.record(&url, &headers);
        match &self.reply {
            Some((status, body)) => Ok(UpstreamResponse {
                status: *status,
                headers: HeaderMap::new(),
                body: body.clone(),
            }),
            None => Err(TransportError::HeadersTooLarge { limit: 0 }),
        }
    }
}
