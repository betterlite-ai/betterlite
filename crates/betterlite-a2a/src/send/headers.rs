use std::collections::BTreeMap;

use betterlite_core::agents::provider::a2a::JsonRpcId;
use betterlite_logging::logging_state::LoggingState;
use secrecy::SecretString;

use crate::logging_context::json_rpc_id_text;

pub(super) const TRACE_ID_HEADER: &str = "X-LiteLLM-Trace-Id";
pub(super) const AGENT_ID_HEADER: &str = "X-LiteLLM-Agent-Id";

// The agent's own headers are applied after the LiteLLM ones, so a name the agent sets wins
#[must_use]
pub(super) fn client_headers(
    trace_id: &str,
    agent_id: Option<&str>,
    agent_extra_headers: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, SecretString> {
    let mut headers = BTreeMap::from([(TRACE_ID_HEADER.to_owned(), SecretString::from(trace_id))]);
    if let Some(id) = agent_id {
        headers.insert(AGENT_ID_HEADER.to_owned(), SecretString::from(id));
    }
    if let Some(extra) = agent_extra_headers {
        headers.extend(
            extra
                .iter()
                .map(|(name, value)| (name.clone(), SecretString::from(value.as_str()))),
        );
    }
    headers
}

#[must_use]
pub(super) fn logging_trace_id(
    logging: Option<&LoggingState>,
    trace_ids: &(dyn Fn() -> String + Sync),
) -> String {
    let trace_id = logging
        .map(|state| state.litellm_trace_id.as_str())
        .filter(|trace_id| !trace_id.is_empty());
    trace_id.map_or_else(trace_ids, str::to_owned)
}

// A streaming request id backs the trace id when no logging object supplied one; LiteLLM skips a falsy id
#[must_use]
pub(super) fn stream_trace_id(
    logging: Option<&LoggingState>,
    request_id: &JsonRpcId,
    trace_ids: &(dyn Fn() -> String + Sync),
) -> String {
    let trace_id = logging
        .map(|state| state.litellm_trace_id.as_str())
        .filter(|trace_id| !trace_id.is_empty());
    if let Some(current) = trace_id {
        return current.to_owned();
    }
    let request_text = json_rpc_id_text(request_id);
    if request_text.is_empty() {
        trace_ids()
    } else {
        request_text
    }
}
