use std::collections::BTreeMap;

use betterlite_anthropic::feature_flags::FeatureParams;
use betterlite_bedrock::event_frames::beta::BetaHeader;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    BetaHeaders {
        model_cost: ParamMap,
        calls: Vec<BetaHeadersCall>,
    },
    ContextFilter {
        cases: Vec<ContextCase>,
    },
    WebSearch {
        request: Object,
        model: String,
    },
    UnsupportedFields {
        cases: Vec<FieldsCase>,
    },
    ProviderIdentity {
        model_cost: ParamMap,
        cases: Vec<IdentityCase>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BetaHeadersCall {
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Json>,
    #[serde(default)]
    pub(super) tools: Vec<Json>,
    #[serde(default)]
    pub(super) mcp_servers: Vec<Json>,
    #[serde(default)]
    pub(super) safeguards: Option<Json>,
    #[serde(default)]
    pub(super) params: FeatureParams,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, BetaHeader>,
    #[serde(default)]
    pub(super) outgoing: Object,
    #[serde(default, rename = "injected")]
    pub(super) clear_thinking_injected: bool,
    #[serde(default)]
    pub(super) subclass_provider: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextCase {
    pub(super) request: Object,
    #[serde(default)]
    pub(super) betas: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldsCase {
    pub(super) request: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) subclass_provider: Option<String>,
}
