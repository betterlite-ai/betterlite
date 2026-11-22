use std::convert::Infallible;

use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::driver::ChatResponseStream;
use crate::api::chat_response_stream_items::sources::EventSources;
use crate::api::responses_chat_options::StreamOptionsSink;
use crate::api::responses_chat_options::capabilities::CompletionBridgeProbe;
use crate::api::responses_chat_options::records::{ChatCompletionRequest, StreamOptions};
use crate::api::responses_chat_output::records::AssembledResponse;

use super::source::ResponsesChunkSource;

// Everything the handler reaches besides the call itself: B014's completion service and the B908 mapping seams
pub struct ResponseChatCall<'deps, Completions> {
    pub completions: &'deps Completions,
    pub limits: ApiLimits,
    pub cache: &'deps DualLocalCache,
    pub parameters: &'deps dyn SupportedParameterSource<Error = Infallible>,
    pub probe: &'deps (dyn CompletionBridgeProbe + Sync),
    pub route_all_openai_to_responses: bool,
}

// The arguments LiteLLM's response_api_handler takes, in the shape both entry points read
pub struct ResponseCall<'call, Context> {
    pub model: &'call str,
    pub request: &'call ResponsesApiRequestParams,
    pub custom_llm_provider: Option<&'call str>,
    pub stream: Option<bool>,
    pub extra_headers: Option<&'call Json>,
    pub kwargs: &'call IndexMap<String, Json>,
    // the handler's future is Send and holds the call across the completion await, so the sink is Send too
    pub logging: Option<&'call mut (dyn StreamOptionsSink + Send)>,
    pub sources: EventSources,
    pub context: Context,
}

// B2410's bridged input and tools beside B908's mapped request, which the session merge and the caller's kwargs patch
pub struct Prepared {
    pub input: Json,
    pub request: Json,
    pub mapped: ChatCompletionRequest,
    pub mapped_json: Map<String, Value>,
}

// One completion call as LiteLLM writes it: the caller's kwargs, then the mapped request, then the bridge key
pub struct CompletionArguments {
    pub fields: IndexMap<String, Value>,
}

// What a streaming iterator names itself after: the async entry reads both from the merged request, the sync one from its arguments
pub struct StreamIdentity {
    pub model: String,
    pub custom_llm_provider: Option<String>,
}

pub enum CallOutcome<Context> {
    Response(Box<AssembledResponse>),
    Stream(Box<ChatResponseStream<ResponsesChunkSource, Context>>),
}

// Everything the conversion step reads, gathered after the completion call so neither branch borrows the call record
pub struct Delivery<Context> {
    pub input: Json,
    pub request: Json,
    pub stream_options: Option<StreamOptions>,
    pub identity: StreamIdentity,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
    pub sources: EventSources,
    pub context: Context,
    pub limits: ApiLimits,
}
