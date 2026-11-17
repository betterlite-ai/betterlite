use betterlite_anthropic::chat_call::EMBEDDING_REPLY;
use betterlite_core::api::anthropic_beta_config::matrix::BetaNode;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

mod failures;
mod fixture;
mod scenarios;
mod support;
mod tests;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scenario {
    MessagesPostRequest,
    BufferedSuccess,
    StreamRequest,
    PreStreamError,
    NativeErrorBody,
    UnsupportedEmbedding,
    ChatFailureProjection,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) scenario: Scenario,
    pub(crate) api_base: String,
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) caller_headers: IndexMap<String, String>,
    pub(crate) api_key: Option<String>,
    pub(crate) acompletion: bool,
    pub(crate) transform_body: IndexMap<String, Json>,
    pub(crate) transform_failure: Option<String>,
    pub(crate) beta_matrix: BetaNode,
    pub(crate) upstream_status: Option<u16>,
    pub(crate) upstream_headers: IndexMap<String, String>,
    pub(crate) upstream_body: Option<String>,
    pub(crate) upstream_lines: Vec<String>,
    pub(crate) failure_message: Option<String>,
    pub(crate) request_header_names: Vec<String>,
    pub(crate) response_header_names: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.scenario {
            Scenario::MessagesPostRequest => scenarios::messages_post_request(input),
            Scenario::BufferedSuccess => scenarios::buffered_success(input),
            Scenario::StreamRequest => scenarios::stream_request(input),
            Scenario::PreStreamError | Scenario::NativeErrorBody => failures::failed_call(input),
            Scenario::UnsupportedEmbedding => Ok(json!({"reply": EMBEDDING_REPLY})),
            Scenario::ChatFailureProjection => failures::chat_failure_projection(input),
        }
    }
}
