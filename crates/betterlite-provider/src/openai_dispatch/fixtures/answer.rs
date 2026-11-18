use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use http::HeaderMap;

use crate::openai_dispatch::transport::ProviderAnswer;
use crate::operation::OperationError;
use crate::prepared::{PreparedKind, PreparedRequest, Upstream};

// The request a prepared call would have sent, in the shape a fixture or an oracle case can compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_limit: Option<usize>,
}

impl RecordedRequest {
    #[must_use]
    pub fn of(prepared: &PreparedRequest) -> Option<Self> {
        let http = match prepared.kind() {
            PreparedKind::Chat(chat)
            | PreparedKind::Stream(chat)
            | PreparedKind::Embedding(chat) => &chat.http,
            PreparedKind::Other => return None,
        };
        Some(Self {
            method: http.method.as_str().to_owned(),
            url: http.url.to_string(),
            headers: http
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().map(str::to_owned).unwrap_or_default(),
                    )
                })
                .collect(),
            body: http.body.clone(),
            body_limit: http.body_limit,
        })
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _value)| header == name)
            .map(|(_header, value)| value.as_str())
    }
}

// The chat and embedding answer a case supplies, with every prepared request it was handed.
pub struct FixtureAnswer {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub sent: Mutex<Vec<RecordedRequest>>,
}

impl FixtureAnswer {
    #[must_use]
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body,
            sent: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ProviderAnswer for FixtureAnswer {
    fn answer(
        &self,
        prepared: &PreparedRequest,
    ) -> impl Future<Output = Result<Upstream, OperationError>> + Send {
        if let Some(record) = RecordedRequest::of(prepared) {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(record);
        }
        ready(Ok(Upstream::Buffered(UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        })))
    }
}
