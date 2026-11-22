use std::collections::BTreeMap;

use betterlite_a2a::send_params::v03::MessageSendParamsV03;
use betterlite_a2a::version_convert::JsonObject;
use betterlite_core::agents::json::AgentJson;
use futures_util::stream::BoxStream;

use super::error::{StreamError, StreamFailure};

pub type JsonStream = BoxStream<'static, Result<AgentJson, StreamError>>;

pub type HookedStream = BoxStream<'static, Result<AgentJson, StreamFailure>>;

// B8589 wraps B4812's normalize_stream_event with the version the agent is served at; the relays own a copy per stream
pub trait StreamEventConverter: Clone + Send + Sync + 'static {
    fn convert(&self, event: JsonObject, request_id: Option<&AgentJson>) -> JsonObject;
}

pub struct StreamOpen {
    pub request_id: AgentJson,
    pub params: MessageSendParamsV03,
    pub api_base: Option<String>,
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    pub agent_id: Option<String>,
    pub metadata: Option<AgentJson>,
    pub proxy_server_request: Option<AgentJson>,
    pub agent_extra_headers: Option<Vec<(String, String)>>,
}

// B8589 wraps B4814's asend_message_streaming; the stream connects when first polled, so every failure is an Err item
pub trait UpstreamAgentStream {
    fn stream(&self, open: StreamOpen) -> JsonStream;
}

// B8589 wraps B9720's shared stream generator: the iterator hook, the per-chunk hook and the failure hook
pub trait StreamHooks {
    fn serve(&self, events: JsonStream) -> HookedStream;
}
