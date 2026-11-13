use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use futures_util::stream::BoxStream;

use super::runtime::RpcMethod;
use super::{AgentCoreError, params::LitellmParams};
use crate::completion_bridge::wire::MessageSendParams;

pub type StreamedEvents<E> = BoxStream<'static, Result<AgentJson, AgentCoreError<E>>>;

pub struct SignedRequestCall<'call> {
    pub request_id: &'call str,
    pub params: &'call MessageSendParams,
    pub litellm_params: &'call LitellmParams,
    pub method: RpcMethod,
    pub agent_extra_headers: Option<&'call BTreeMap<String, String>>,
}

pub struct HandlerCall<'call> {
    pub request_id: &'call str,
    pub params: &'call MessageSendParams,
    pub litellm_params: LitellmParams,
    pub agent_extra_headers: Option<&'call BTreeMap<String, String>>,
}

pub struct AgentCoreHandler<R, S> {
    pub runtime: R,
    pub ids: S,
}
