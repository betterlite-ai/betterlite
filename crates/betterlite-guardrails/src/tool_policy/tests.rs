#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use indexmap::IndexMap;
use rstest::rstest;

use super::error::ToolPolicyError;
use super::metadata::{permission_ids_from_data, request_route_from_data};
use super::names::{request_tool_names, response_tool_names, tool_call_names_from_messages};
use super::{
    DEFAULT_GUARDRAIL_NAME, RequestToolNames, ToolPolicyGuardrail, ToolPolicyLookup,
    initialize_tool_policy,
};
use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailToolParam};
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::GuardrailSettings;

type PolicyCall = (Vec<String>, Option<String>, Option<String>);

struct FakeStore {
    initialized: bool,
    input: BTreeMap<String, String>,
    output: BTreeMap<String, String>,
    blocked: BTreeMap<String, Vec<String>>,
    calls: Mutex<Vec<PolicyCall>>,
    output_calls: Mutex<Vec<String>>,
}

impl ToolPolicyLookup for FakeStore {
    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn effective_policies(
        &self,
        tool_names: &[String],
        key_permission_id: Option<&str>,
        team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        self.calls.lock().unwrap().push((
            tool_names.to_vec(),
            key_permission_id.map(str::to_owned),
            team_permission_id.map(str::to_owned),
        ));
        let mut blocked: Vec<&str> = Vec::new();
        for id in [key_permission_id, team_permission_id]
            .into_iter()
            .flatten()
        {
            if let Some(listed) = self.blocked.get(id.trim()) {
                blocked.extend(listed.iter().map(String::as_str));
            }
        }
        tool_names
            .iter()
            .map(|name| {
                let policy = if blocked.contains(&name.as_str()) {
                    "blocked"
                } else {
                    self.input.get(name).map_or("untrusted", String::as_str)
                };
                (name.clone(), policy.to_owned())
            })
            .collect()
    }

    fn output_policy(&self, tool_name: &str) -> String {
        self.output_calls.lock().unwrap().push(tool_name.to_owned());
        self.output
            .get(tool_name)
            .cloned()
            .unwrap_or_else(|| "untrusted".to_owned())
    }
}

struct StaticNames {
    reply: Vec<String>,
    routes: Mutex<Vec<String>>,
}

impl RequestToolNames for StaticNames {
    fn names(&self, route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        self.routes.lock().unwrap().push(route.to_owned());
        self.reply.clone()
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn names_of(values: &[&str]) -> GuardrailValue {
    GuardrailValue::Array(values.iter().map(|value| text(value)).collect())
}

fn pairs(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn fake(
    initialized: bool,
    input: &[(&str, &str)],
    output: &[(&str, &str)],
    blocked: &[(&str, &[&str])],
) -> Arc<FakeStore> {
    Arc::new(FakeStore {
        initialized,
        input: pairs(input),
        output: pairs(output),
        blocked: blocked
            .iter()
            .map(|(id, names)| {
                let listed = names.iter().map(|name| (*name).to_owned()).collect();
                ((*id).to_owned(), listed)
            })
            .collect(),
        calls: Mutex::new(Vec::new()),
        output_calls: Mutex::new(Vec::new()),
    })
}

fn store(initialized: bool, input: &[(&str, &str)], output: &[(&str, &str)]) -> Arc<FakeStore> {
    fake(initialized, input, output, &[])
}

fn static_names(reply: &[&str]) -> Arc<StaticNames> {
    Arc::new(StaticNames {
        reply: reply.iter().map(|name| (*name).to_owned()).collect(),
        routes: Mutex::new(Vec::new()),
    })
}

fn guardrail(store: Arc<FakeStore>, names: Arc<StaticNames>) -> ToolPolicyGuardrail {
    ToolPolicyGuardrail::new(GuardrailSettings::default(), true, store, names).unwrap()
}

fn body(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

fn inputs(json: &str) -> GuardrailApiInputs {
    serde_json::from_str(json).unwrap()
}

fn params(json: &str) -> LitellmParams {
    LitellmParams::try_from(serde_json::from_str::<OpaqueParams>(json).unwrap()).unwrap()
}

fn tools(json: &str) -> Vec<GuardrailToolParam> {
    serde_json::from_str(json).unwrap()
}

fn calls(json: &str) -> Vec<GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

async fn apply(
    guardrail: &ToolPolicyGuardrail,
    inputs: GuardrailApiInputs,
    request_data: &str,
    input_type: GuardrailInputType,
) -> Result<GuardrailApiInputs, GuardrailFailure> {
    guardrail
        .apply_guardrail(inputs, &mut body(request_data), input_type, None)
        .await
}

fn http_detail(failure: GuardrailFailure) -> (u16, IndexMap<String, GuardrailValue>) {
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("expected an HTTP object failure");
    };
    (status, detail)
}

fn key_list(detail: &IndexMap<String, GuardrailValue>) -> Vec<&str> {
    detail.keys().map(String::as_str).collect()
}

fn plain_settings() -> GuardrailSettings {
    initialized(
        None,
        true,
        r#"{"guardrail": "tool_policy", "mode": "pre_call"}"#,
    )
    .unwrap()
    .base
    .settings
}

#[tokio::test]
async fn blocked_request_tool_returns_400_with_ordered_detail() {
    let guardrail = guardrail(
        store(true, &[("dangerous", "blocked")], &[]),
        static_names(&[]),
    );
    let outcome = apply(
        &guardrail,
        inputs(r#"{"tools": [{"type": "function", "function": {"name": "dangerous"}}]}"#),
        "{}",
        GuardrailInputType::Request,
    )
    .await;
    let (status, detail) = http_detail(outcome.unwrap_err());
    assert_eq!(status, 400);
    assert_eq!(key_list(&detail), ["error", "blocked_tools", "message"]);
    assert_eq!(detail["error"], text("Violated tool policy"));
    assert_eq!(detail["blocked_tools"], names_of(&["dangerous"]));
    assert_eq!(
        detail["message"],
        text("Tool(s) ['dangerous'] are blocked by policy.")
    );
}

#[tokio::test]
async fn blocked_keeps_request_order_and_duplicates() {
    let guardrail = guardrail(
        store(true, &[("first", "blocked"), ("second", "blocked")], &[]),
        static_names(&[]),
    );
    let outcome = apply(
        &guardrail,
        inputs(
            r#"{"tools": [{"type": "function", "function": {"name": "first"}},
                           {"type": "function", "function": {"name": "fine"}},
                           {"type": "function", "function": {"name": "second"}},
                           {"type": "function", "function": {"name": "first"}}]}"#,
        ),
        "{}",
        GuardrailInputType::Request,
    )
    .await;
    let (_, detail) = http_detail(outcome.unwrap_err());
    assert_eq!(
        detail["blocked_tools"],
        names_of(&["first", "second", "first"])
    );
    assert_eq!(
        detail["message"],
        text("Tool(s) ['first', 'second', 'first'] are blocked by policy.")
    );
}

#[tokio::test]
async fn blocked_by_key_and_team_permission_ids() {
    let store = fake(
        true,
        &[],
        &[],
        &[("key-1", &["first"]), ("team-1", &["second"])],
    );
    let guardrail = guardrail(Arc::clone(&store), static_names(&[]));
    let outcome = apply(
        &guardrail,
        inputs(
            r#"{"tools": [{"type": "function", "function": {"name": "first"}},
                           {"type": "function", "function": {"name": "fine"}},
                           {"type": "function", "function": {"name": "second"}}]}"#,
        ),
        r#"{"metadata": {"user_api_key_object_permission_id": "key-1",
                          "user_api_key_team_object_permission_id": "team-1"}}"#,
        GuardrailInputType::Request,
    )
    .await;
    let (status, detail) = http_detail(outcome.unwrap_err());
    assert_eq!(status, 400);
    assert_eq!(detail["blocked_tools"], names_of(&["first", "second"]));
}

#[tokio::test]
async fn blocked_response_tool_call_is_checked_before_the_trust_chain() {
    let guardrail = guardrail(
        store(
            true,
            &[("dangerous", "blocked")],
            &[("writer", "untrusted")],
        ),
        static_names(&[]),
    );
    let outcome = apply(
        &guardrail,
        inputs(r#"{"tool_calls": [{"id": "c1", "function": {"name": "dangerous"}}]}"#),
        r#"{"messages": [{"role": "assistant", "tool_calls": [{"id": "c1", "function": {"name": "writer"}}]},
                         {"role": "tool", "tool_call_id": "c1", "content": "data"}]}"#,
        GuardrailInputType::Response,
    )
    .await;
    let (status, detail) = http_detail(outcome.unwrap_err());
    assert_eq!(status, 400);
    assert_eq!(key_list(&detail), ["error", "blocked_tools", "message"]);
}

#[rstest]
#[case::other_case("Blocked")]
#[case::upper_case("BLOCKED")]
#[case::trusted("trusted")]
#[case::untrusted("untrusted")]
#[tokio::test]
async fn blocked_requires_the_exact_policy_text(#[case] policy: &str) {
    let guardrail = guardrail(
        store(true, &[("dangerous", policy)], &[]),
        static_names(&[]),
    );
    let inputs = inputs(r#"{"tools": [{"type": "function", "function": {"name": "dangerous"}}]}"#);
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        "{}",
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
}

#[test]
fn names_request_tools_read_function_names_in_order() {
    let names = static_names(&[]);
    let collected = request_tool_names(
        Some(
            tools(
                r#"[{"type": "function", "function": {"name": "a"}},
                    {"type": "function", "function": {"name": "b"}}]"#,
            )
            .as_slice(),
        ),
        names.as_ref(),
        &body("{}"),
    );
    assert_eq!(collected, ["a", "b"]);
    assert!(names.routes.lock().unwrap().is_empty());
}

#[test]
fn names_request_skips_tools_without_a_function_or_name() {
    let collected = request_tool_names(
        Some(
            tools(
                r#"[{"type": "function"},
                    {"type": "function", "function": {}},
                    {"type": "function", "function": {"name": ""}},
                    {"type": "custom", "function": {"name": "kept"}},
                    {"type": "function", "function": {"name": "also"}}]"#,
            )
            .as_slice(),
        ),
        static_names(&[]).as_ref(),
        &body("{}"),
    );
    assert_eq!(collected, ["kept", "also"]);
}

#[test]
fn names_route_fallback_runs_when_tools_give_no_names() {
    let names = static_names(&["from_route"]);
    let collected = request_tool_names(
        None,
        names.as_ref(),
        &body(r#"{"metadata": {"user_api_key_request_route": "/v1/chat/completions"}}"#),
    );
    assert_eq!(collected, ["from_route"]);
    assert_eq!(
        names.routes.lock().unwrap().as_slice(),
        ["/v1/chat/completions"]
    );
}

#[test]
fn names_route_fallback_needs_a_route() {
    let names = static_names(&["from_route"]);
    let collected = request_tool_names(
        Some(tools(r#"[{"type": "function"}]"#).as_slice()),
        names.as_ref(),
        &body(r#"{"metadata": {}}"#),
    );
    assert!(collected.is_empty());
    assert!(names.routes.lock().unwrap().is_empty());
}

#[test]
fn names_route_fallback_skips_an_empty_route() {
    let names = static_names(&["from_route"]);
    let collected = request_tool_names(
        None,
        names.as_ref(),
        &body(r#"{"metadata": {"user_api_key_request_route": ""}}"#),
    );
    assert!(collected.is_empty());
    assert!(names.routes.lock().unwrap().is_empty());
}

#[test]
fn names_tools_keep_the_route_extractor_out() {
    let names = static_names(&["from_route"]);
    let collected = request_tool_names(
        Some(tools(r#"[{"type": "function", "function": {"name": "direct"}}]"#).as_slice()),
        names.as_ref(),
        &body(r#"{"metadata": {"user_api_key_request_route": "/v1/chat/completions"}}"#),
    );
    assert_eq!(collected, ["direct"]);
    assert!(names.routes.lock().unwrap().is_empty());
}

#[test]
fn names_response_reads_tool_call_function_names() {
    let collected = response_tool_names(Some(
        calls(
            r#"[{"id": "c1", "function": {"name": "a"}},
                {"id": "c2", "function": {"name": "a"}},
                {"id": "c3", "function": {"name": "b"}}]"#,
        )
        .as_slice(),
    ));
    assert_eq!(collected, ["a", "a", "b"]);
}

#[test]
fn names_response_skips_calls_without_a_name() {
    let collected = response_tool_names(Some(
        calls(
            r#"[{"id": "c1", "function": {}},
                "not-a-call",
                {"id": "c4", "function": {"name": "kept"}}]"#,
        )
        .as_slice(),
    ));
    assert_eq!(collected, ["kept"]);
}

#[test]
fn names_tool_call_names_from_messages_keeps_the_last_id() {
    let names = tool_call_names_from_messages(&calls(
        r#"[{"role": "assistant", "tool_calls": [{"id": "c1", "function": {"name": "first"}},
                                                   {"id": "c1", "function": {"name": "second"}},
                                                   {"id": "", "function": {"name": "empty"}},
                                                   {"id": "c2", "function": {}}]},
            "not-a-message",
            {"role": "user", "tool_calls": [{"id": "c3", "function": {"name": "user"}}]},
            {"role": "assistant", "tool_calls": "not-a-list"}]"#,
    ));
    assert_eq!(
        names,
        BTreeMap::from([("c1".to_owned(), "second".to_owned())])
    );
}

#[tokio::test]
async fn passthrough_no_tool_names_returns_the_inputs() {
    let store = store(true, &[("a", "blocked")], &[]);
    let guardrail = guardrail(Arc::clone(&store), static_names(&[]));
    let inputs = inputs(r#"{"texts": ["hello"], "model": "gpt-4"}"#);
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        "{}",
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
    assert!(store.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn passthrough_uninitialized_store_returns_the_inputs() {
    let store = store(false, &[("a", "blocked")], &[("a", "untrusted")]);
    let guardrail = guardrail(Arc::clone(&store), static_names(&[]));
    let inputs = inputs(r#"{"tools": [{"type": "function", "function": {"name": "a"}}]}"#);
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        "{}",
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
    assert!(store.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn passthrough_allowed_tools_return_the_inputs() {
    let store = store(
        true,
        &[("a", "trusted"), ("b", "untrusted")],
        &[("a", "trusted")],
    );
    let guardrail = guardrail(Arc::clone(&store), static_names(&[]));
    let inputs = inputs(
        r#"{"tools": [{"type": "function", "function": {"name": "a"}},
                       {"type": "function", "function": {"name": "b"}}]}"#,
    );
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        "{}",
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
    assert!(store.output_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn passthrough_response_without_trusted_tools_skips_the_chain() {
    let store = store(true, &[("reader", "untrusted")], &[("writer", "untrusted")]);
    let guardrail = guardrail(Arc::clone(&store), static_names(&[]));
    let inputs = inputs(r#"{"tool_calls": [{"id": "c1", "function": {"name": "reader"}}]}"#);
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        r#"{"messages": [{"role": "assistant", "tool_calls": [{"id": "c1", "function": {"name": "writer"}}]},
                         {"role": "tool", "tool_call_id": "c1", "content": "data"}]}"#,
        GuardrailInputType::Response,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
    assert!(store.output_calls.lock().unwrap().is_empty());
}

const TRUST_MESSAGES: &str = r#"{"messages": [{"role": "assistant", "tool_calls": [{"id": "c1", "function": {"name": "writer"}}]},
                                             {"role": "tool", "tool_call_id": "c1", "content": "data"}]}"#;

fn reader_response() -> GuardrailApiInputs {
    inputs(r#"{"tool_calls": [{"id": "c9", "function": {"name": "reader"}}]}"#)
}

#[tokio::test]
async fn trust_response_fails_on_an_untrusted_source() {
    let guardrail = guardrail(
        store(true, &[("reader", "trusted")], &[("writer", "untrusted")]),
        static_names(&[]),
    );
    let outcome = apply(
        &guardrail,
        reader_response(),
        TRUST_MESSAGES,
        GuardrailInputType::Response,
    )
    .await;
    let (status, detail) = http_detail(outcome.unwrap_err());
    assert_eq!(status, 400);
    assert_eq!(
        key_list(&detail),
        ["error", "blocked_tools", "untrusted_sources", "message"]
    );
    assert_eq!(detail["blocked_tools"], names_of(&["reader"]));
    assert_eq!(detail["untrusted_sources"], names_of(&["writer"]));
    assert_eq!(
        detail["message"],
        text(
            "reader requires trusted input but conversation contains untrusted output from writer."
        )
    );
}

#[tokio::test]
async fn trust_unknown_output_policy_is_untrusted() {
    let guardrail = guardrail(
        store(true, &[("reader", "trusted")], &[]),
        static_names(&[]),
    );
    let outcome = apply(
        &guardrail,
        reader_response(),
        TRUST_MESSAGES,
        GuardrailInputType::Response,
    )
    .await;
    let (_, detail) = http_detail(outcome.unwrap_err());
    assert_eq!(detail["untrusted_sources"], names_of(&["writer"]));
}

#[tokio::test]
async fn trust_sources_are_listed_once_in_first_seen_order() {
    let guardrail = guardrail(
        store(
            true,
            &[("reader", "trusted")],
            &[("second", "untrusted"), ("first", "untrusted")],
        ),
        static_names(&[]),
    );
    let outcome = apply(
        &guardrail,
        reader_response(),
        r#"{"messages": [{"role": "assistant", "tool_calls": [{"id": "c1", "function": {"name": "second"}},
                                                              {"id": "c2", "function": {"name": "first"}}]},
                         {"role": "tool", "tool_call_id": "c1", "content": "one"},
                         {"role": "tool", "tool_call_id": "c2", "content": "two"},
                         {"role": "tool", "tool_call_id": "c1", "content": "again"}]}"#,
        GuardrailInputType::Response,
    )
    .await;
    let (_, detail) = http_detail(outcome.unwrap_err());
    assert_eq!(detail["untrusted_sources"], names_of(&["second", "first"]));
    assert_eq!(
        detail["message"],
        text(
            "reader requires trusted input but conversation contains untrusted output from second, first."
        )
    );
}

#[rstest]
#[case::unknown_call_id(
    r#"{"messages": [{"role": "tool", "tool_call_id": "unknown", "content": "data"}]}"#
)]
#[case::missing_call_id(
    r#"{"messages": [{"role": "assistant", "tool_calls": [{"id": "c1", "function": {"name": "writer"}}]},
                     {"role": "tool", "content": "data"},
                     {"role": "user", "tool_call_id": "c1", "content": "not a tool message"},
                     "not-a-message"]}"#
)]
#[tokio::test]
async fn trust_unknown_or_missing_call_ids_are_ignored(#[case] request_data: &str) {
    let guardrail = guardrail(
        store(true, &[("reader", "trusted")], &[]),
        static_names(&[]),
    );
    let inputs = reader_response();
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        request_data,
        GuardrailInputType::Response,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
}

#[tokio::test]
async fn trust_trusted_output_source_passes() {
    let guardrail = guardrail(
        store(true, &[("reader", "trusted")], &[("writer", "trusted")]),
        static_names(&[]),
    );
    let inputs = reader_response();
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        TRUST_MESSAGES,
        GuardrailInputType::Response,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
}

#[tokio::test]
async fn trust_request_side_is_not_checked() {
    let store = store(true, &[("reader", "trusted")], &[("writer", "untrusted")]);
    let guardrail = guardrail(Arc::clone(&store), static_names(&[]));
    let inputs = inputs(r#"{"tools": [{"type": "function", "function": {"name": "reader"}}]}"#);
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        TRUST_MESSAGES,
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
    assert!(store.output_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn trust_requires_the_exact_trusted_text() {
    let guardrail = guardrail(
        store(true, &[("reader", "TRUSTED")], &[("writer", "untrusted")]),
        static_names(&[]),
    );
    let inputs = reader_response();
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        TRUST_MESSAGES,
        GuardrailInputType::Response,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
}

#[rstest]
#[case::absent(r#"{"model": "gpt-4"}"#)]
#[case::null(r#"{"messages": null}"#)]
#[case::not_a_list(r#"{"messages": "text"}"#)]
#[tokio::test]
async fn trust_missing_or_null_messages_are_empty(#[case] request_data: &str) {
    let guardrail = guardrail(
        store(true, &[("reader", "trusted")], &[("writer", "untrusted")]),
        static_names(&[]),
    );
    let inputs = reader_response();
    let outcome = apply(
        &guardrail,
        inputs.clone(),
        request_data,
        GuardrailInputType::Response,
    )
    .await;
    assert_eq!(outcome.unwrap(), inputs);
}

#[rstest]
#[case::nested_auth_wins(
    r#"{"metadata": {"user_api_key_auth": {"object_permission_id": "key-id", "team_object_permission_id": "team-id"},
                      "user_api_key_object_permission_id": "flat-key",
                      "user_api_key_team_object_permission_id": "flat-team"}}"#,
    (Some("key-id"), Some("team-id"))
)]
#[case::auth_without_ids_uses_flat(
    r#"{"metadata": {"user_api_key_auth": {},
                      "user_api_key_object_permission_id": "flat-key",
                      "user_api_key_team_object_permission_id": "flat-team"}}"#,
    (Some("flat-key"), Some("flat-team"))
)]
#[case::null_auth_ids_use_flat(
    r#"{"metadata": {"user_api_key_auth": {"object_permission_id": null, "team_object_permission_id": null},
                      "user_api_key_object_permission_id": "flat-key"}}"#,
    (Some("flat-key"), None)
)]
#[case::litellm_metadata_first(
    r#"{"litellm_metadata": {"user_api_key_object_permission_id": "first"},
        "metadata": {"user_api_key_object_permission_id": "second"}}"#,
    (Some("first"), None)
)]
#[case::empty_first_bucket_falls_to_second(
    r#"{"litellm_metadata": {},
        "metadata": {"user_api_key_team_object_permission_id": "team"}}"#,
    (None, Some("team"))
)]
#[case::one_null_id_keeps_the_other_side(
    r#"{"metadata": {"user_api_key_object_permission_id": null,
                      "user_api_key_team_object_permission_id": "team"}}"#,
    (None, Some("team"))
)]
#[case::trimmed_and_blank(
    r#"{"metadata": {"user_api_key_object_permission_id": "  key \n",
                      "user_api_key_team_object_permission_id": "   "}}"#,
    (Some("key"), Some(""))
)]
#[case::numbers_become_text(
    r#"{"metadata": {"user_api_key_object_permission_id": 7,
                      "user_api_key_team_object_permission_id": 42}}"#,
    (Some("7"), Some("42"))
)]
#[case::empty_body("{}", (None, None))]
fn ids_read_the_first_bucket_that_names_one(
    #[case] request_data: &str,
    #[case] expected: (Option<&str>, Option<&str>),
) {
    let ids = permission_ids_from_data(&body(request_data));
    assert_eq!(
        ids,
        (expected.0.map(str::to_owned), expected.1.map(str::to_owned))
    );
}

#[rstest]
#[case::metadata_is_a_string(r#"{"metadata": "text"}"#)]
#[case::bucket_misses_the_keys(r#"{"metadata": {"other": 1}}"#)]
#[case::empty_buckets(r#"{"litellm_metadata": {}, "metadata": {}}"#)]
fn ids_returns_none_without_a_readable_bucket(#[case] request_data: &str) {
    assert_eq!(permission_ids_from_data(&body(request_data)), (None, None));
}

#[rstest]
#[case::flat_empty_text(r#"{"metadata": {"user_api_key_object_permission_id": ""}}"#)]
#[case::auth_empty_text(
    r#"{"metadata": {"user_api_key_auth": {"object_permission_id": ""},
                      "user_api_key_object_permission_id": ""}}"#
)]
fn ids_empty_text_counts_as_missing(#[case] request_data: &str) {
    assert_eq!(permission_ids_from_data(&body(request_data)), (None, None));
}

#[rstest]
#[case::top_level_wins(
    r#"{"user_api_key_request_route": "/v1/chat/completions",
        "metadata": {"user_api_key_request_route": "/v1/responses"}}"#,
    Some("/v1/chat/completions")
)]
#[case::metadata_before_litellm_metadata(
    r#"{"metadata": {"user_api_key_request_route": "/v1/chat/completions"},
        "litellm_metadata": {"user_api_key_request_route": "/v1/responses"}}"#,
    Some("/v1/chat/completions")
)]
#[case::empty_metadata_falls_to_litellm_metadata(
    r#"{"metadata": {},
        "litellm_metadata": {"user_api_key_request_route": "/v1/responses"}}"#,
    Some("/v1/responses")
)]
#[case::empty_top_level_falls_through(
    r#"{"user_api_key_request_route": "",
        "litellm_metadata": {"user_api_key_request_route": "/v1/responses"}}"#,
    Some("/v1/responses")
)]
#[case::absent("{}", None)]
#[case::no_route_key(r#"{"metadata": {"other": 1}}"#, None)]
#[case::route_not_text(r#"{"metadata": {"user_api_key_request_route": 7}}"#, None)]
fn ids_route_is_the_first_non_empty_text_route(
    #[case] request_data: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        request_route_from_data(&body(request_data)),
        expected.map(str::to_owned)
    );
}

#[tokio::test]
async fn ids_reach_the_policy_lookup_with_the_tool_names() {
    let store = store(true, &[], &[]);
    let guardrail = guardrail(Arc::clone(&store), static_names(&[]));
    apply(
        &guardrail,
        inputs(
            r#"{"tools": [{"type": "function", "function": {"name": "a"}},
                           {"type": "function", "function": {"name": "b"}}]}"#,
        ),
        r#"{"metadata": {"user_api_key_auth": {"object_permission_id": "key-id", "team_object_permission_id": "team-id"}}}"#,
        GuardrailInputType::Request,
    )
    .await
    .unwrap();
    let calls = store.calls.lock().unwrap();
    assert_eq!(
        calls.as_slice(),
        [(
            vec!["a".to_owned(), "b".to_owned()],
            Some("key-id".to_owned()),
            Some("team-id".to_owned())
        )]
    );
}

#[test]
fn constructor_defaults_the_supported_hooks() {
    let guardrail = guardrail(store(true, &[], &[]), static_names(&[]));
    assert_eq!(
        guardrail.base.settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall
        ])
    );
    assert_eq!(guardrail.base.mode, None);
}

#[test]
fn constructor_keeps_a_given_hook_list() {
    let settings = GuardrailSettings {
        supported_event_hooks: Some(vec![GuardrailEventHooks::PreCall]),
        ..plain_settings()
    };
    let guardrail =
        ToolPolicyGuardrail::new(settings, true, store(true, &[], &[]), static_names(&[])).unwrap();
    assert_eq!(
        guardrail.base.settings.supported_event_hooks,
        Some(vec![GuardrailEventHooks::PreCall])
    );
}

#[test]
fn constructor_rejects_a_mode_outside_the_hook_list() {
    let settings = GuardrailSettings {
        supported_event_hooks: Some(vec![GuardrailEventHooks::PreCall]),
        event_hook: Some(LitellmParamsMode::One("post_call".to_owned())),
        ..plain_settings()
    };
    let outcome =
        ToolPolicyGuardrail::new(settings, true, store(true, &[], &[]), static_names(&[]));
    assert!(matches!(
        outcome,
        Err(ToolPolicyError::Base(
            GuardrailBaseError::UnsupportedEventHook { .. }
        ))
    ));
}

fn initialized(
    guardrail_name: Option<&str>,
    strict_modes: bool,
    json: &str,
) -> Result<ToolPolicyGuardrail, ToolPolicyError> {
    initialize_tool_policy(
        &params(json),
        guardrail_name,
        strict_modes,
        store(true, &[], &[]),
        static_names(&[]),
    )
}

#[rstest]
#[case::default_name(None, DEFAULT_GUARDRAIL_NAME)]
#[case::given_name(Some("team_tools"), "team_tools")]
#[test]
fn constructor_initialize_names_the_guardrail(#[case] given: Option<&str>, #[case] expected: &str) {
    let guardrail = initialized(
        given,
        true,
        r#"{"guardrail": "tool_policy", "mode": "post_call"}"#,
    )
    .unwrap();
    assert_eq!(
        guardrail.base.settings.guardrail_name.as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::default_on_true(
    r#"{"guardrail": "tool_policy", "mode": "post_call", "default_on": true}"#,
    true
)]
#[case::default_on_absent(r#"{"guardrail": "tool_policy", "mode": "post_call"}"#, false)]
#[test]
fn constructor_initialize_reads_mode_and_default_on(#[case] json: &str, #[case] expected: bool) {
    let settings = params(json);
    let guardrail = initialize_tool_policy(
        &settings,
        None,
        true,
        store(true, &[], &[]),
        static_names(&[]),
    )
    .unwrap();
    assert_eq!(guardrail.base.settings.event_hook, Some(settings.mode));
    assert_eq!(guardrail.base.settings.default_on, expected);
}

#[test]
fn constructor_initialize_ignores_the_other_params() {
    let guardrail = initialized(
        None,
        true,
        r#"{"guardrail": "tool_policy", "mode": "post_call", "mask_request_content": true,
            "violation_message_template": "blocked {default_message}", "run_in_parallel": true,
            "end_session_after_n_fails": 3}"#,
    )
    .unwrap();
    assert!(!guardrail.base.settings.mask_request_content);
    assert_eq!(guardrail.base.settings.violation_message_template, None);
    assert!(!guardrail.base.settings.run_in_parallel);
    assert_eq!(guardrail.base.settings.end_session_after_n_fails, None);
}

#[test]
fn constructor_initialize_accepts_logging_only_for_apply_guardrails() {
    let guardrail = initialized(
        None,
        true,
        r#"{"guardrail": "tool_policy", "mode": "logging_only"}"#,
    );
    guardrail.unwrap();
}

#[test]
fn constructor_initialize_rejects_an_unsupported_mode() {
    let outcome = initialized(
        None,
        true,
        r#"{"guardrail": "tool_policy", "mode": "pre_mcp_call"}"#,
    );
    assert!(matches!(
        outcome,
        Err(ToolPolicyError::Base(
            GuardrailBaseError::UnsupportedEventHook { .. }
        ))
    ));
}

#[test]
fn constructor_initialize_without_strict_modes_keeps_an_unsupported_mode() {
    let guardrail = initialized(
        None,
        false,
        r#"{"guardrail": "tool_policy", "mode": "pre_mcp_call"}"#,
    );
    assert_eq!(
        guardrail.unwrap().base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_mcp_call".to_owned()))
    );
}
