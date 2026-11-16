pub mod apply;
pub mod attempts;
pub mod blocked_turns;
pub mod envelope;
pub mod failure_policy;
pub mod responses;
pub mod settings;
pub mod transport;
pub mod v3_prepare;

#[path = "delivery/tests/fixtures.rs"]
mod fixtures;

#[path = "delivery/api_key_validation/tests.rs"]
mod api_key_validation;
#[path = "delivery/api_version_validation/tests.rs"]
mod api_version_validation;
#[path = "delivery/block_exception/tests.rs"]
mod block_exception;
#[path = "delivery/error_classification/tests.rs"]
mod error_classification;
#[path = "delivery/fallback_behavior/tests.rs"]
mod fallback_behavior;
#[path = "delivery/fallback_validation/tests.rs"]
mod fallback_validation;
#[path = "delivery/field_clamping/tests.rs"]
mod field_clamping;
#[path = "delivery/field_defaults/tests.rs"]
mod field_defaults;
#[path = "delivery/format_hint_validation/tests.rs"]
mod format_hint_validation;
#[path = "delivery/header_composition/tests.rs"]
mod header_composition;
#[path = "delivery/intervention_decision/tests.rs"]
mod intervention_decision;
#[path = "delivery/payload_delivery/tests.rs"]
mod payload_delivery;
#[path = "delivery/response_parsing/tests.rs"]
mod response_parsing;
#[path = "delivery/retry_configuration/tests.rs"]
mod retry_configuration;
#[path = "delivery/v3_key_detection/tests.rs"]
mod v3_key_detection;
#[path = "delivery/v3_replay_blocking/tests.rs"]
mod v3_replay_blocking;
#[path = "delivery/v3_replay_cache/tests.rs"]
mod v3_replay_cache;
#[path = "delivery/v3_session_resolution/tests.rs"]
mod v3_session_resolution;
#[path = "delivery/verbose_logging/tests.rs"]
mod verbose_logging;
#[path = "delivery/webhook_url/tests.rs"]
mod webhook_url;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_prompts::response_tool_calls::structured::{
    RouteCallTypes, StructuredMessageHandler,
};

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::straiker::v3::answer_shape::AnthropicTranslator;
use crate::straiker::v3::prompt_text::PromptTokenDecoder;
use crate::straiker_webhook::context::ProviderNames;

pub const GUARDRAIL_NAME: &str = "straiker";
pub const DEFAULT_BLOCK_MESSAGE: &str = "Content violates policy";
pub const DEFAULT_MAX_RETRIES: i64 = 2;
pub const DEFAULT_INITIAL_BACKOFF: f64 = 0.1;
pub const DEFAULT_MAX_BACKOFF: f64 = 2.0;
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 512 * 1024;
pub const WEBHOOK_PATH: &str = "/api/v1/detect/webhook";
pub const LITELLM_VERSION: &str = "1.105.1";
// LiteLLM's InMemoryCache(V3_BLOCKED_TURN_MEMORY, V3_BLOCKED_TURN_TTL_SECONDS) with its default item size
pub const BLOCKED_TURN_TTL_SECONDS: f64 = 86_400.0;
pub const BLOCKED_TURN_ITEM_KIB: i64 = 1024;

pub struct StraikerDelivery<Post, Delay> {
    pub settings: settings::StraikerSettings,
    pub transport: Post,
    pub delay: Delay,
    pub blocked_turns: blocked_turns::BlockedTurns,
}

pub struct ApplyRequest<'request> {
    pub inputs: GuardrailApiInputs,
    pub request_data: &'request mut BTreeMap<String, GuardrailValue>,
    pub input_type: GuardrailInputType,
    pub call: &'request DeliveryCall<'request>,
}

pub struct DeliveryCall<'call> {
    pub call_type: Option<&'call str>,
    pub litellm_call_id: Option<&'call str>,
    pub litellm_trace_id: Option<&'call str>,
    pub provider_names: &'call (dyn ProviderNames + Sync),
    pub routes: &'call (dyn RouteCallTypes<CallType> + Sync),
    pub mappings: &'call [(
        &'call [CallType],
        &'call (dyn StructuredMessageHandler + Sync),
    )],
    pub decoder: &'call (dyn PromptTokenDecoder + Sync),
    pub translator: &'call (dyn AnthropicTranslator + Sync),
}

#[must_use]
pub const fn phase_name(input_type: GuardrailInputType) -> &'static str {
    match input_type {
        GuardrailInputType::Request => "request",
        GuardrailInputType::Response => "response",
    }
}
