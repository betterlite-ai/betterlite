use std::sync::Arc;

use crate::api::passthrough_call::{PassthroughCallArgs, PassthroughReply};
use crate::api::video_router_calls::route::VideoRouteView;
use crate::dispatch::llm_passthrough::route::PassthroughRouteInput;
use crate::dispatch::llm_passthrough::{BlockedModels, PassthroughCallPort};

pub type LlmPassthroughResult = PassthroughReply;

pub struct LlmPassthroughRequest {
    pub args: Box<PassthroughCallArgs>,
    pub route: PassthroughRouteInput,
    pub router: Option<Arc<dyn PassthroughCallPort>>,
    pub view: Option<Arc<dyn VideoRouteView + Send + Sync>>,
    pub blocked: Arc<dyn BlockedModels>,
    pub direct: Arc<dyn PassthroughCallPort>,
}
