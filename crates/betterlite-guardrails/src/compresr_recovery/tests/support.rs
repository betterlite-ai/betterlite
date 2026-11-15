use std::time::Duration;

use betterlite_context::agentic_callback::{AgenticCall, AgenticLogging, AgenticLoopCall};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::value::RawValue;

use crate::compresr_recovery::error::RecoveryError;
use crate::compresr_recovery::plan::{PlanRequest, build_plan, should_run};
use crate::compresr_recovery::store::{Originals, RecoveryStore, StoreKey};
use crate::compresr_recovery::tools::{CallKind, RetrieveCall};
use crate::primitives::json_value::JsonValue;

pub(super) const SCOPE: &str = "scope-a";
pub(super) const CALL_ID: &str = "call-1";
pub(super) const NOW: Duration = Duration::from_secs(100);
pub(super) const NO_CAP: usize = 0;
pub(super) const HASH_ONE: &str = "47080470c1aeb2a10e9df746";
pub(super) const RETRIEVE_TOOL: &str =
    r#"[{"type":"function","function":{"name":"compresr_retrieve"}}]"#;

pub(super) fn raw(json: &str) -> Box<RawValue> {
    serde_json::from_str(json).unwrap()
}

pub(super) fn map(json: &str) -> JsonMap {
    serde_json::from_str(json).unwrap()
}

pub(super) fn messages(json: &str) -> Vec<FollowupMessage> {
    serde_json::from_str(json).unwrap()
}

pub(super) fn value(json: &str) -> JsonValue {
    serde_json::from_str(json).unwrap()
}

pub(super) fn originals(pairs: &[(&str, &str)]) -> Originals {
    pairs
        .iter()
        .map(|(hash, text)| ((*hash).to_owned(), (*text).to_owned()))
        .collect()
}

pub(super) fn key(scope: &str, call_id: &str) -> StoreKey {
    StoreKey::resolve(Some(call_id), scope).unwrap()
}

pub(super) fn seeded(pairs: &[(&str, &str)]) -> RecoveryStore {
    let store = RecoveryStore::default();
    store.store(&key(SCOPE, CALL_ID), originals(pairs), NOW, NO_CAP);
    store
}

pub(super) fn retrieve_call(id: &str, hash: &str) -> RetrieveCall {
    let mut arguments = Object::new();
    arguments.insert("hash".to_owned(), Json::Text(hash.to_owned()));
    RetrieveCall {
        id: Some(id.to_owned()),
        kind: CallKind::Function,
        name: Some("compresr_retrieve".to_owned()),
        arguments,
    }
}

pub(super) fn chat_response(hash: &str) -> String {
    r#"{"choices":[{"message":{"content":"thinking","tool_calls":[{"id":"call_1","type":"function","function":{"name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"}},{"id":"call_2","type":"function","function":{"name":"search","arguments":"{}"}}]}}]}"#
        .replace("HASH", hash)
}

pub(super) fn anthropic_response(hash: &str) -> String {
    r#"{"content":[{"type":"text","text":"let me look"},{"type":"tool_use","id":"toolu_1","name":"compresr_retrieve","input":{"hash":"HASH"}},{"type":"tool_use","id":"toolu_2","name":"search","input":{}}]}"#
        .replace("HASH", hash)
}

pub(super) fn responses_response(hash: &str) -> String {
    r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"checking"}]},{"type":"function_call","call_id":"call_1","name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"},{"type":"function_call","call_id":"call_2","name":"search","arguments":"{}"}]}"#
        .replace("HASH", hash)
}

pub(super) struct Setup {
    pub(super) scope: &'static str,
    pub(super) logging: Option<AgenticLogging>,
    pub(super) optional_params: JsonMap,
    pub(super) kwargs: JsonMap,
    pub(super) tools: Option<JsonMap>,
    pub(super) marker: Option<&'static str>,
    pub(super) now: Duration,
}

pub(super) fn setup() -> Setup {
    Setup {
        scope: SCOPE,
        logging: Some(AgenticLogging {
            call_id: Some(CALL_ID.to_owned()),
            agentic_loop_params: None,
        }),
        optional_params: JsonMap::new(),
        kwargs: JsonMap::new(),
        tools: None,
        marker: None,
        now: NOW,
    }
}

pub(super) fn gate_tools(response: &RawValue) -> JsonMap {
    let tools = messages(RETRIEVE_TOOL);
    let kwargs = JsonMap::new();
    let call = AgenticCall {
        response,
        model: "gpt-4o",
        messages: &[],
        tools: Some(&tools),
        stream: false,
        custom_llm_provider: "openai",
        kwargs: &kwargs,
    };
    should_run(&call).unwrap().tools
}

pub(super) fn plan_with(
    store: &RecoveryStore,
    response_json: &str,
    setup: &Setup,
) -> Result<AgenticLoopPlan, RecoveryError> {
    let response = raw(response_json);
    let tools = setup.tools.clone().unwrap_or_else(|| gate_tools(&response));
    let thread = messages(r#"[{"role":"user","content":"hi"}]"#);
    let request = PlanRequest {
        call: AgenticLoopCall {
            tools: &tools,
            model: "gpt-4o",
            messages: &thread,
            response: &response,
            optional_params: &setup.optional_params,
            logging: setup.logging.as_ref(),
            stream: false,
            kwargs: &setup.kwargs,
        },
        caller_scope: setup.scope,
        own_marker: setup.marker,
        now: setup.now,
    };
    build_plan(store, &request)
}

pub(super) fn plan_messages(plan: &AgenticLoopPlan) -> JsonValue {
    let patch = plan.request_patch.as_ref().unwrap();
    let encoded = serde_json::to_string(patch.messages.as_ref().unwrap()).unwrap();
    value(&encoded)
}
