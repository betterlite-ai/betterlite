use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use serde::{Deserialize, Serialize};

use crate::adaptive_classification::request_type::RequestType;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterKind {
    Complexity,
    Adaptive,
    Quality,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingCause {
    Bandit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingDecision {
    pub router_model_name: String,
    pub router_type: RouterKind,
    pub routed_model: String,
    pub cause: RoutingCause,
    pub request_type: RequestType,
}

pub struct PreRoutingRequest<'messages, 'request> {
    pub messages: Option<&'messages [Message]>,
    pub headers: Option<&'request Json>,
    pub metadata: &'request mut Option<Json>,
}

pub struct PreRoutingResponse<'messages> {
    pub model: String,
    pub messages: Option<&'messages [Message]>,
    pub routing_decision: RoutingDecision,
}
