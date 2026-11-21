#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_guardrail_policy::GuardrailPolicySettings;
use betterlite_core::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use betterlite_models::decisions::Json;
use betterlite_policy::registry::{PolicyRegistry, RawPolicy};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::error::HookPayloadError;
use super::from_llm::{llm_result_to_during_call, llm_result_to_pre_call};
use super::kwargs::{
    PreCallDecision, mcp_request_from_kwargs, mcp_response_to_kwargs, parse_pre_mcp_call_response,
};
use super::to_llm::{AuthGuardrails, McpGuardrailRequest, MessageRole, mcp_to_llm_request};
use super::{
    GuardrailHookData, GuardrailOutcome, HookMessage, McpDuringCallResponse, McpHookKwargs,
    McpPreCallRequest, McpPreCallResponse, McpToolCall, ToolArguments,
};

fn arguments(source: &str) -> ToolArguments {
    serde_json::from_str(source).unwrap()
}

fn kwargs(source: &str) -> McpHookKwargs {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn convert_with(
    policies: &PolicyRegistry,
    tool_arguments: &ToolArguments,
    hook_kwargs: &McpHookKwargs,
    premium_user: bool,
) -> Result<McpGuardrailRequest, HookPayloadError> {
    let guardrails = AuthGuardrails {
        policies,
        settings: GuardrailPolicySettings {
            premium_user,
            limits: ApiLimits::default(),
        },
    };
    let call = McpToolCall {
        tool_name: "search",
        arguments: tool_arguments,
    };
    mcp_to_llm_request(call, hook_kwargs, &guardrails)
}

fn convert(hook_kwargs: &str, premium_user: bool) -> Result<McpGuardrailRequest, HookPayloadError> {
    convert_with(
        &PolicyRegistry::default(),
        &arguments("{}"),
        &kwargs(hook_kwargs),
        premium_user,
    )
}

fn metadata_entry(request: &McpGuardrailRequest, key: &str) -> Option<Json> {
    request.metadata.get(key).cloned()
}

fn answer(content: Option<&str>) -> GuardrailOutcome {
    GuardrailOutcome::Data(Box::new(GuardrailHookData {
        messages: vec![HookMessage {
            content: content.map(str::to_owned),
        }],
        modified_arguments: None,
        extra_headers: None,
    }))
}

fn checked<Response>(
    convert: fn(&GuardrailOutcome, McpToolCall<'_>) -> Result<Option<Response>, HookPayloadError>,
    outcome: &GuardrailOutcome,
    original: &str,
) -> Option<Response> {
    let tool_arguments = arguments(original);
    let call = McpToolCall {
        tool_name: "search",
        arguments: &tool_arguments,
    };
    convert(outcome, call).unwrap()
}

fn proceeds_with(modified: &str) -> Option<McpPreCallResponse> {
    Some(McpPreCallResponse {
        should_proceed: true,
        modified_arguments: Some(arguments(modified)),
        error_message: None,
        hidden_params: IndexMap::new(),
    })
}

fn stopped_with(message: &str) -> Option<McpPreCallResponse> {
    Some(McpPreCallResponse {
        should_proceed: false,
        modified_arguments: None,
        error_message: Some(message.to_owned()),
        hidden_params: IndexMap::new(),
    })
}

fn stopped_during(message: &str) -> Option<McpDuringCallResponse> {
    Some(McpDuringCallResponse {
        should_continue: false,
        error_message: Some(message.to_owned()),
        hidden_params: IndexMap::new(),
    })
}

#[test]
fn the_message_names_the_tool_its_description_and_its_arguments() {
    let request = convert_with(
        &PolicyRegistry::default(),
        &arguments(r#"{"q": "docs\nfaq", "limit": 2}"#),
        &kwargs(r#"{"mcp_tool_description": "Search the docs"}"#),
        false,
    )
    .unwrap();
    assert_eq!(request.messages.len(), 1);
    let message = request.messages.first().unwrap();
    assert_eq!(message.role, MessageRole::User);
    assert_eq!(
        message.content,
        "Tool: search\nDescription: Search the docs\nArguments: {\"q\":\"docs\\nfaq\",\"limit\":2}"
    );
    assert_eq!(
        request.mcp_tool_description.as_deref(),
        Some("Search the docs")
    );
    assert_eq!(request.mcp_tool_name, "search");
    assert_eq!(
        request.mcp_arguments,
        arguments(r#"{"q": "docs\nfaq", "limit": 2}"#)
    );
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"mcp_tool_description": null}"#)]
#[case::empty(r#"{"mcp_tool_description": ""}"#)]
fn an_empty_description_adds_no_line_and_no_field(#[case] hook_kwargs: &str) {
    let request = convert(hook_kwargs, false).unwrap();
    assert_eq!(
        request.messages.first().unwrap().content,
        "Tool: search\nArguments: {}"
    );
    assert_eq!(request.mcp_tool_description, None);
}

#[rstest]
#[case::context_model(r#"{"guardrail_context": {"model": "ctx"}, "model": "kw"}"#, "ctx")]
#[case::context_without_model(r#"{"guardrail_context": {"metadata": {}}, "model": "kw"}"#, "kw")]
#[case::null_context_model(r#"{"guardrail_context": {"model": null}, "model": "kw"}"#, "kw")]
#[case::empty_context_model(r#"{"guardrail_context": {"model": ""}, "model": "kw"}"#, "")]
#[case::null_context(r#"{"guardrail_context": null}"#, "mcp-tool-call")]
#[case::nothing("{}", "mcp-tool-call")]
fn the_model_comes_from_the_context_then_the_kwargs(
    #[case] hook_kwargs: &str,
    #[case] expected: &str,
) {
    assert_eq!(convert(hook_kwargs, false).unwrap().model, expected);
}

const FULL_KWARGS: &str = r#"{
    "server_name": "github",
    "mcp_rate_limit_server_name": "gh-alias",
    "user_api_key_user_id": "u1",
    "user_api_key_team_id": "t1",
    "user_api_key_end_user_id": "e1",
    "user_api_key_hash": "h1",
    "user_api_key_request_route": "/mcp/tools/call",
    "incoming_bearer_token": "inbound",
    "mcp_input_schema": {"type": "object"}
}"#;

#[test]
fn the_ids_route_server_and_token_are_copied() {
    let request = convert(FULL_KWARGS, false).unwrap();
    assert_eq!(request.user_api_key_user_id.as_deref(), Some("u1"));
    assert_eq!(request.user_api_key_team_id.as_deref(), Some("t1"));
    assert_eq!(request.user_api_key_end_user_id.as_deref(), Some("e1"));
    assert_eq!(request.user_api_key_hash.as_deref(), Some("h1"));
    assert_eq!(
        request.user_api_key_request_route.as_deref(),
        Some("/mcp/tools/call")
    );
    assert_eq!(request.mcp_server_name.as_deref(), Some("gh-alias"));
    assert_eq!(
        request
            .incoming_bearer_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("inbound".to_owned())
    );
    assert_eq!(
        request.mcp_input_schema,
        Some(arguments(r#"{"type": "object"}"#))
    );
}

#[test]
fn the_metadata_carries_the_three_ids() {
    let request = convert(FULL_KWARGS, false).unwrap();
    assert_eq!(
        Json::Object(request.metadata),
        json(
            r#"{"headers": {}, "user_api_key_user_id": "u1", "user_api_key_team_id": "t1",
                "user_api_key_end_user_id": "e1", "guardrails": []}"#
        )
    );
}

#[test]
fn missing_ids_stay_null_and_headers_an_empty_object() {
    let request = convert(r#"{"headers": null}"#, false).unwrap();
    assert_eq!(request.user_api_key_user_id, None);
    assert_eq!(request.user_api_key_hash, None);
    assert_eq!(request.mcp_server_name, None);
    assert_eq!(request.mcp_input_schema, None);
    assert!(request.incoming_bearer_token.is_none());
    assert_eq!(
        Json::Object(request.metadata),
        json(
            r#"{"headers": {}, "user_api_key_user_id": null, "user_api_key_team_id": null,
                "user_api_key_end_user_id": null, "guardrails": []}"#
        )
    );
}

#[test]
fn parent_metadata_is_copied_without_its_guardrails_and_with_the_new_headers() {
    let request = convert(
        r#"{
            "guardrail_context": {"metadata": {"trace": "t1", "guardrails": ["pg", "pg"], "headers": {"old": "1"}}},
            "headers": {"x-request": "1"},
            "user_api_key_user_id": "u1"
        }"#,
        false,
    )
    .unwrap();
    assert_eq!(
        Json::Object(request.metadata),
        json(
            r#"{"trace": "t1", "headers": {"x-request": "1"}, "user_api_key_user_id": "u1",
                "user_api_key_team_id": null, "user_api_key_end_user_id": null, "guardrails": ["pg"]}"#
        )
    );
}

#[test]
fn auth_guardrails_come_first_and_repeats_are_dropped() {
    let request = convert(
        r#"{
            "guardrail_context": {"metadata": {"guardrails": ["tg", "pg"]}},
            "user_api_key_auth": {
                "metadata": {"guardrails": ["kg"]},
                "team_metadata": {"guardrails": ["tg", "kg"], "team": "t"},
                "project_metadata": {"guardrails": ["prj"]}
            }
        }"#,
        true,
    )
    .unwrap();
    assert_eq!(
        metadata_entry(&request, "guardrails"),
        Some(json(r#"["kg", "tg", "prj", "pg"]"#))
    );
    assert_eq!(
        metadata_entry(&request, "user_api_key_metadata"),
        Some(json(r#"{"guardrails": ["kg"]}"#))
    );
    assert_eq!(
        metadata_entry(&request, "user_api_key_team_metadata"),
        Some(json(r#"{"guardrails": ["tg", "kg"], "team": "t"}"#))
    );
}

#[test]
fn an_auth_record_without_guardrails_needs_no_license() {
    let request = convert(r#"{"user_api_key_auth": {"metadata": {}}}"#, false).unwrap();
    assert_eq!(metadata_entry(&request, "guardrails"), Some(json("[]")));
    assert_eq!(
        metadata_entry(&request, "user_api_key_metadata"),
        Some(json("{}"))
    );
    assert_eq!(
        metadata_entry(&request, "user_api_key_team_metadata"),
        Some(Json::Null)
    );
}

#[test]
fn no_auth_record_copies_no_key_or_team_metadata() {
    let request = convert(r#"{"user_api_key_user_id": "u1"}"#, false).unwrap();
    assert_eq!(metadata_entry(&request, "user_api_key_metadata"), None);
    assert_eq!(metadata_entry(&request, "user_api_key_team_metadata"), None);
}

#[test]
fn auth_guardrails_without_a_license_are_refused() {
    let error = convert(
        r#"{"user_api_key_auth": {"metadata": {"guardrails": ["kg"]}}}"#,
        false,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        HookPayloadError::Guardrails(RequestGuardrailPolicyError::NotPremium)
    ));
}

#[test]
fn auth_policies_resolve_through_the_policy_registry() {
    let mut policies = PolicyRegistry::default();
    let raw: BTreeMap<String, RawPolicy> =
        serde_json::from_str(r#"{"p1": {"guardrails": {"add": ["pii"]}}}"#).unwrap();
    policies.load_policies(raw).unwrap();
    let request = convert_with(
        &policies,
        &arguments("{}"),
        &kwargs(r#"{"user_api_key_auth": {"metadata": {"policies": ["p1"]}}}"#),
        true,
    )
    .unwrap();
    assert_eq!(
        metadata_entry(&request, "guardrails"),
        Some(json(r#"["pii"]"#))
    );
    assert_eq!(
        metadata_entry(&request, "applied_policies"),
        Some(json(r#"["p1"]"#))
    );
}

#[rstest]
#[case::metadata_not_an_object(r#"{"metadata": "kg"}"#, "metadata", "guardrails")]
#[case::guardrails_not_a_list(
    r#"{"team_metadata": {"guardrails": "kg"}}"#,
    "team_metadata",
    "guardrails"
)]
#[case::policy_not_a_string(
    r#"{"project_metadata": {"policies": [1]}}"#,
    "project_metadata",
    "policies"
)]
fn auth_metadata_of_the_wrong_type_names_the_field(
    #[case] auth: &str,
    #[case] expected_field: &str,
    #[case] expected_key: &str,
) {
    let error = convert(&format!(r#"{{"user_api_key_auth": {auth}}}"#), true).unwrap_err();
    let HookPayloadError::AuthMetadata { field, key } = &error else {
        panic!("expected an auth metadata error, got {error:?}");
    };
    assert_eq!((*field, *key), (expected_field, expected_key));
}

#[test]
fn parent_guardrails_that_are_not_a_list_are_refused() {
    let error = convert(
        r#"{"guardrail_context": {"metadata": {"guardrails": "pg"}}}"#,
        false,
    )
    .unwrap_err();
    assert!(matches!(error, HookPayloadError::GuardrailList));
}

#[rstest]
#[case::raised(GuardrailOutcome::Raised("guardrail said no".to_owned()))]
#[case::text(GuardrailOutcome::Text("guardrail said no".to_owned()))]
fn an_error_or_a_string_stops_the_call(#[case] outcome: GuardrailOutcome) {
    assert_eq!(
        checked(llm_result_to_pre_call, &outcome, "{}"),
        stopped_with("guardrail said no")
    );
    assert_eq!(
        checked(llm_result_to_during_call, &outcome, "{}"),
        stopped_during("guardrail said no")
    );
}

#[test]
fn an_unchanged_message_leaves_the_call_alone() {
    let outcome = answer(Some("Tool: search\nArguments: {\"q\":\"docs\"}"));
    assert_eq!(
        checked(llm_result_to_pre_call, &outcome, r#"{"q": "docs"}"#),
        None
    );
    assert_eq!(
        checked(llm_result_to_during_call, &outcome, r#"{"q": "docs"}"#),
        None
    );
}

#[test]
fn an_answer_without_messages_leaves_the_call_alone() {
    let outcome = GuardrailOutcome::Data(Box::new(GuardrailHookData {
        messages: Vec::new(),
        modified_arguments: None,
        extra_headers: None,
    }));
    assert_eq!(checked(llm_result_to_pre_call, &outcome, "{}"), None);
    assert_eq!(checked(llm_result_to_during_call, &outcome, "{}"), None);
}

#[rstest]
#[case::blocked("Tool: search\nArguments: BLOCKED by policy")]
#[case::violation("Policy Violation detected")]
#[case::empty("")]
fn pre_call_blocks_on_its_words_or_empty_content(#[case] content: &str) {
    assert_eq!(
        checked(llm_result_to_pre_call, &answer(Some(content)), "{}"),
        stopped_with("Content blocked by guardrail")
    );
}

#[test]
fn pre_call_blocks_a_message_without_content() {
    assert_eq!(
        checked(llm_result_to_pre_call, &answer(None), "{}"),
        stopped_with("Content blocked by guardrail")
    );
}

#[test]
fn masked_json_arguments_replace_the_originals() {
    let outcome = answer(Some(
        "Tool: search\nArguments: {\"q\": \"[MASKED]\", \"limit\": 3}",
    ));
    assert_eq!(
        checked(
            llm_result_to_pre_call,
            &outcome,
            r#"{"q": "secret", "limit": 2}"#
        ),
        proceeds_with(r#"{"q": "[MASKED]", "limit": 3}"#)
    );
}

#[test]
fn the_arguments_line_is_found_after_the_content_is_trimmed() {
    let outcome = answer(Some("  Arguments: {\"q\": \"[MASKED]\"}  \r\n"));
    assert_eq!(
        checked(llm_result_to_pre_call, &outcome, r#"{"q": "secret"}"#),
        proceeds_with(r#"{"q": "[MASKED]"}"#)
    );
}

#[test]
fn an_echoed_described_message_counts_as_changed() {
    let outcome = answer(Some(
        "Tool: search\nDescription: Search the docs\nArguments: {\"q\":\"docs\"}",
    ));
    assert_eq!(
        checked(llm_result_to_pre_call, &outcome, r#"{"q": "docs"}"#),
        proceeds_with(r#"{"q": "docs"}"#)
    );
}

#[test]
fn the_manual_fallback_replaces_only_named_string_arguments() {
    let outcome = answer(Some(
        "Tool: search\nArguments: {'EMAIL': '[EMAIL]', 'limit': '9', 'note': ''}",
    ));
    assert_eq!(
        checked(
            llm_result_to_pre_call,
            &outcome,
            r#"{"email": "a@b.co", "limit": 5, "note": "keep", "name": "Jo"}"#
        ),
        proceeds_with(r#"{"email": "[EMAIL]", "limit": 5, "note": "keep", "name": "Jo"}"#)
    );
}

#[test]
fn the_manual_fallback_trims_but_keeps_what_the_pattern_reads() {
    let outcome = answer(Some("Tool: search\nArguments: {email: [EMAIL] , name: J*}"));
    assert_eq!(
        checked(
            llm_result_to_pre_call,
            &outcome,
            r#"{"email": "a@b.co", "name": "Jo"}"#
        ),
        proceeds_with(r#"{"email": "[EMAIL]", "name": "J*}"}"#)
    );
}

#[rstest]
#[case::no_arguments_line("Tool: search\nArgs: {}")]
#[case::indented_arguments_line("Tool: search\n  Arguments: {}")]
#[case::json_null("Tool: search\nArguments: null")]
#[case::json_list("Tool: search\nArguments: [1]")]
fn an_answer_without_readable_arguments_leaves_the_call_alone(#[case] content: &str) {
    assert_eq!(
        checked(
            llm_result_to_pre_call,
            &answer(Some(content)),
            r#"{"q": "docs"}"#
        ),
        None
    );
}

#[rstest]
#[case::blocked(
    "Tool: search\nArguments: Blocked",
    "Content blocked by guardrail during execution"
)]
#[case::empty("", "Content blocked by guardrail during execution")]
#[case::violation("Policy violation", "Content modified by guardrail during execution")]
#[case::masked(
    "Tool: search\nArguments: {\"q\": \"[MASKED]\"}",
    "Content modified by guardrail during execution"
)]
fn a_changed_answer_stops_the_running_call(#[case] content: &str, #[case] message: &str) {
    assert_eq!(
        checked(
            llm_result_to_during_call,
            &answer(Some(content)),
            r#"{"q": "docs"}"#
        ),
        stopped_during(message)
    );
}

#[test]
fn a_running_call_stops_on_a_message_without_content() {
    assert_eq!(
        checked(llm_result_to_during_call, &answer(None), "{}"),
        stopped_during("Content blocked by guardrail during execution")
    );
}

fn original_request() -> McpPreCallRequest {
    McpPreCallRequest {
        tool_name: "search".to_owned(),
        arguments: arguments(r#"{"q": "docs"}"#),
        server_name: None,
        user_api_key_auth: IndexMap::new(),
        hidden_params: IndexMap::new(),
    }
}

#[rstest]
#[case::none_modified(None, r#"{"q": "docs"}"#)]
#[case::modified(Some(r#"{"q": "[MASKED]"}"#), r#"{"q": "[MASKED]"}"#)]
#[case::emptied(Some("{}"), "{}")]
fn the_parsed_response_keeps_the_original_arguments_unless_modified(
    #[case] modified: Option<&str>,
    #[case] expected: &str,
) {
    let response = McpPreCallResponse {
        should_proceed: false,
        modified_arguments: modified.map(arguments),
        error_message: Some("why".to_owned()),
        hidden_params: arguments(r#"{"model_id": "m"}"#),
    };
    assert_eq!(
        parse_pre_mcp_call_response(response, &original_request()),
        PreCallDecision {
            should_proceed: false,
            modified_arguments: arguments(expected),
            error_message: Some("why".to_owned()),
            hidden_params: arguments(r#"{"model_id": "m"}"#),
        }
    );
}

#[test]
fn the_request_object_reads_the_kwargs_with_their_defaults() {
    assert_eq!(
        mcp_request_from_kwargs(&kwargs(r#"{"name": null, "arguments": null}"#)),
        McpPreCallRequest {
            tool_name: String::new(),
            arguments: IndexMap::new(),
            server_name: None,
            user_api_key_auth: IndexMap::new(),
            hidden_params: IndexMap::new(),
        }
    );
    assert_eq!(
        mcp_request_from_kwargs(&kwargs(
            r#"{"name": "search", "arguments": {"q": "docs"}, "server_name": "github",
                "user_api_key_auth": {"user_id": "u1"}}"#
        )),
        McpPreCallRequest {
            tool_name: "search".to_owned(),
            arguments: arguments(r#"{"q": "docs"}"#),
            server_name: Some("github".to_owned()),
            user_api_key_auth: arguments(r#"{"user_id": "u1"}"#),
            hidden_params: IndexMap::new(),
        }
    );
}

#[test]
fn kwargs_refuse_a_key_nobody_reads() {
    let error = serde_json::from_str::<McpHookKwargs>(r#"{"nme": "search"}"#).unwrap_err();
    assert!(error.to_string().contains("unknown field `nme`"));
}

fn hook_kwargs_with_headers() -> McpHookKwargs {
    kwargs(
        r#"{"name": "search", "arguments": {"q": "docs"}, "extra_headers": {"a": "1", "b": "1"}}"#,
    )
}

#[test]
fn no_response_data_returns_the_kwargs_unchanged() {
    let unchanged = mcp_response_to_kwargs(None, hook_kwargs_with_headers());
    assert_eq!(unchanged.arguments, arguments(r#"{"q": "docs"}"#));
    assert_eq!(
        unchanged.extra_headers,
        Some(headers(&[("a", "1"), ("b", "1")]))
    );
}

#[test]
fn response_data_replaces_the_arguments_and_merges_the_headers() {
    let data = GuardrailHookData {
        messages: Vec::new(),
        modified_arguments: Some(arguments(r#"{"q": "[MASKED]"}"#)),
        extra_headers: Some(headers(&[("b", "2"), ("c", "2")])),
    };
    let modified = mcp_response_to_kwargs(Some(&data), hook_kwargs_with_headers());
    assert_eq!(modified.arguments, arguments(r#"{"q": "[MASKED]"}"#));
    assert_eq!(
        modified.extra_headers,
        Some(headers(&[("a", "1"), ("b", "2"), ("c", "2")]))
    );
}

#[test]
fn response_data_without_changes_keeps_arguments_and_headers() {
    let data = GuardrailHookData::default();
    let kept = mcp_response_to_kwargs(Some(&data), hook_kwargs_with_headers());
    assert_eq!(kept.arguments, arguments(r#"{"q": "docs"}"#));
    assert_eq!(kept.extra_headers, Some(headers(&[("a", "1"), ("b", "1")])));
    let bare = mcp_response_to_kwargs(Some(&data), kwargs("{}"));
    assert_eq!(bare.extra_headers, None);
}

#[test]
fn headers_land_on_kwargs_that_had_none() {
    let data = GuardrailHookData {
        messages: Vec::new(),
        modified_arguments: None,
        extra_headers: Some(headers(&[("authorization", "Bearer signed")])),
    };
    let modified = mcp_response_to_kwargs(Some(&data), kwargs(r#"{"name": "search"}"#));
    assert_eq!(
        modified.extra_headers,
        Some(headers(&[("authorization", "Bearer signed")]))
    );
}
