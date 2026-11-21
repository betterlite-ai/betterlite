use std::sync::Arc;

use crate::api::passthrough_call::{PassthroughCallArgs, PassthroughReply};
use crate::api::video_router_calls::route::VideoRouteView;
use crate::dispatch::llm_passthrough::route::PassthroughRouteInput;
use crate::dispatch::llm_passthrough::{
    BlockedModels, PassthroughDirectCall, PassthroughRouterCall,
};

pub type LlmPassthroughResult = PassthroughReply;

pub struct LlmPassthroughRequest {
    pub args: PassthroughCallArgs,
    pub route: PassthroughRouteInput,
    pub router: Option<Arc<dyn PassthroughRouterCall>>,
    pub view: Option<Arc<dyn VideoRouteView>>,
    pub blocked: Arc<dyn BlockedModels>,
    pub direct: Arc<dyn PassthroughDirectCall>,
}
