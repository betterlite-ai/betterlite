use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;

use scenarios::{
    handshake_error, non_http_error, response_headers, stream_flag_and_context, stream_post,
    sync_status_guard,
};

mod calls;
mod fixture;
mod scenarios;
mod support;
mod tests;

pub(crate) const MAX_EVENT_BYTES: usize = 0x0010_0000;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scenario {
    StreamPost,
    HandshakeError,
    #[serde(rename = "non_http_error_500")]
    NonHttpError500,
    SyncStatusGuard,
    StreamFlagAndContext,
    ResponseHeaders,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) scenario: Scenario,
    pub(crate) api_base: String,
    pub(crate) messages: Json,
    pub(crate) model: String,
    pub(crate) json_mode: bool,
    pub(crate) headers: IndexMap<String, String>,
    pub(crate) request_header_names: Vec<String>,
    pub(crate) response_header_names: Vec<String>,
    pub(crate) data: String,
    pub(crate) request_data: IndexMap<String, Json>,
    pub(crate) speed: Option<String>,
    pub(crate) reverse_map: IndexMap<String, String>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) upstream_status: Option<u16>,
    pub(crate) upstream_headers: IndexMap<String, String>,
    pub(crate) upstream_lines: Vec<String>,
    pub(crate) upstream_body: Option<String>,
    pub(crate) failure_message: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.scenario {
            Scenario::StreamPost => stream_post(input),
            Scenario::HandshakeError => handshake_error(input),
            Scenario::NonHttpError500 => non_http_error(input),
            Scenario::SyncStatusGuard => sync_status_guard(input),
            Scenario::StreamFlagAndContext => stream_flag_and_context(input),
            Scenario::ResponseHeaders => response_headers(input),
        }
    }
}
