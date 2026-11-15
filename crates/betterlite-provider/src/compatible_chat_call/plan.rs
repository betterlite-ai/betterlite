use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_openai::compatible_line_stream::Mode;
use betterlite_openai::compatible_wire::{NEW_ID, ReplyOptions, header_map, system_clock};
use betterlite_openai::openai_base_config::Headers;
use http::Method;
use indexmap::IndexMap;
use url::Url;

use super::error::CompatibleChatError;
use super::execution::TimeoutFacts;
use super::stream::StreamingDecoder;

pub struct ChatPlan<'source> {
    pub mode: Mode,
    pub stream: bool,
    pub fake_stream: bool,
    pub url: String,
    pub headers: Headers,
    pub body: IndexMap<String, Json>,
    pub messages: Json,
    pub provider: String,
    pub base_model: Option<String>,
    pub json_mode: bool,
    // the timeout kwarg the source hands its send, if it hands one
    pub send_timeout: Option<TimeoutFacts>,
    // the timeout the blocking handler is constructed with; its post passes none
    pub construct_timeout: Option<TimeoutFacts>,
    pub decoder: Option<&'source dyn StreamingDecoder>,
}

impl ChatPlan<'_> {
    #[must_use]
    pub const fn streams(&self) -> bool {
        self.stream
    }

    // the source's send reads the whole body unless it streams, and the blocking fake stream never streams
    #[must_use]
    pub const fn buffered(&self) -> bool {
        !self.stream || (matches!(self.mode, Mode::Sync) && self.fake_stream)
    }

    // what the send carries: the timeout it forwards, else the one its handler was built with, else httpx's default
    #[must_use]
    pub fn effective_timeout(&self) -> TimeoutFacts {
        self.send_timeout
            .or(self.construct_timeout)
            .unwrap_or(TimeoutFacts::DEFAULT)
    }

    // the async stream send forwards no timeout of its own, so its client's own default is the only one in play
    #[must_use]
    pub fn request_timeout(&self) -> Option<TimeoutFacts> {
        if self.mode == Mode::Async && self.stream {
            return None;
        }
        Some(self.effective_timeout())
    }

    // the async stream send carries no source timeout, so the transport's own stream default bounds it
    #[must_use]
    pub fn deadline(&self) -> Duration {
        if let Some(facts) = self.send_timeout.or(self.construct_timeout) {
            return facts.total;
        }
        if self.mode == Mode::Async && self.stream {
            return TransportLimits::DEFAULT.read_timeout;
        }
        TimeoutFacts::DEFAULT.total
    }

    pub(crate) fn reply_options(&self) -> ReplyOptions<'_> {
        ReplyOptions {
            json_mode: self.json_mode,
            provider: Some(self.provider.as_str()),
            base_model: self.base_model.as_deref(),
            identifier: NEW_ID,
            clock: system_clock,
        }
    }

    pub(crate) fn request(&self) -> Result<OutboundRequest, CompatibleChatError> {
        Ok(OutboundRequest {
            method: Method::POST,
            url: Url::parse(&self.url).map_err(CompatibleChatError::Url)?,
            query: Vec::new(),
            headers: header_map(&self.headers).map_err(CompatibleChatError::Header)?,
            body: RequestBody::Json(python_json::dumps_object(&self.body).into_bytes()),
            deadline: self.deadline(),
        })
    }

    // make_call and make_sync_call wrap nothing: only the nonstream functions map a failed send
    pub(crate) fn send_failure(&self, error: TransportError) -> CompatibleChatError {
        if self.streams() {
            return CompatibleChatError::Transport(error);
        }
        if matches!(
            error,
            TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
        ) {
            CompatibleChatError::Timeout(error)
        } else {
            CompatibleChatError::Transport(error)
        }
    }
}
