use std::fmt::Display;
use std::future::{Future, ready};

use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_foundry::mai_images::reply::{ImageLogging, PostCall};
use betterlite_foundry::passthrough::logging::FoundryPassthroughLogging;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::passthrough::transformation::types::{PassthroughLogging, RelayCallType};
use betterlite_oracle_helper_api::error::HelperError;
use http::HeaderMap;
use tokio::runtime::{Builder, Runtime};

// Reached only if an operation and the result variant it reads back ever disagree
pub(super) const OTHER: &str = "the dispatch returned another outcome";

pub(super) fn failure<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| failure("RuntimeError", error))
}

// The reply logging is a production input the compared answer does not read
#[derive(Default)]
pub(super) struct Recorder;

impl ImageLogging for Recorder {
    fn post_call(&mut self, _call: PostCall<'_>) {}
}

#[derive(Default)]
pub(super) struct RelayLog {
    pub(super) call_type: Option<String>,
}

impl PassthroughLogging for RelayLog {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_type = Some(call_type.as_str().to_owned());
    }
}

impl FoundryPassthroughLogging for RelayLog {
    fn set_ocr_call_type(&mut self) {
        self.call_type = Some(String::from("aocr"));
    }
}

// The messages rows compare endpoints and the request, so the count transport never runs
pub(super) struct NoTransport;

impl CountTokensTransport for NoTransport {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(UpstreamResponse {
            status: 500,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }))
    }
}
