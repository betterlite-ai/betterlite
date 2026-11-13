use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::headers::Header;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
#[error("request to {url} failed: {reason}")]
pub struct TransportError {
    pub url: String,
    pub reason: String,
}

// A non-2xx answer is an Ok(HttpResponse); only a request that never got an answer is an Err.
pub trait Transport {
    fn get(&self, url: &str, headers: &[Header]) -> Result<HttpResponse, TransportError>;
    fn post_form(
        &self,
        url: &str,
        headers: &[Header],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError>;
    fn post_json(
        &self,
        url: &str,
        headers: &[Header],
        payload: &IndexMap<String, Json>,
    ) -> Result<HttpResponse, TransportError>;
}

// uuid.uuid4() in GalileoObserve._record_to_v2_trace, injected so a test or an oracle run can fix the ids.
pub trait TraceIds {
    fn next_id(&self) -> String;
}
