#![cfg(test)]
mod support;

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::params::Stop;
use betterlite_prompts::message_content::{Content, Message};
use betterlite_router::routing_timeouts::completion::TimeoutValue;
use betterlite_router::routing_timeouts::seconds::Seconds;
use indexmap::IndexMap as JsonObject;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;

use super::call::PreparedCompletion;
use super::error::CompletionRequestError;
use super::ports::PromptOutcome;
use super::{prepare_acompletion, prepare_completion};
use support::{
    Fixture, call_of, hook_parameters, json, lookups, mapping_with, message, parameters,
};

async fn run(fixture: &Fixture) -> Result<PreparedCompletion, CompletionRequestError> {
    prepare_completion(&fixture.args, &fixture.settings, &fixture.ports()).await
}

async fn run_async(fixture: &Fixture) -> Result<PreparedCompletion, CompletionRequestError> {
    Box::pin(prepare_acompletion(
        &fixture.args,
        &fixture.settings,
        &fixture.ports(),
    ))
    .await
}

fn header_of(call: &super::call::PreparedCall, name: &str) -> Option<String> {
    match call.headers.get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

#[tokio::test]
async fn order_steps_follow_litellms_sequence() {
    let fixture = Fixture::new();
    let outcome = run(&fixture).await.unwrap();
    assert!(matches!(outcome, PreparedCompletion::Dispatch(_)));
    assert_eq!(
        fixture.events(),
        vec![
            "seed",
            "should_run",
            "resolve:gpt-4o",
            "bridge_info",
            "mapping",
            "log",
            "bridge_info"
        ]
    );
}

#[tokio::test]
async fn order_prompt_hook_model_reaches_provider_resolution() {
    let mut fixture = Fixture::new();
    fixture.prompt.should_run = true;
    let outcome = PromptOutcome {
        model: String::from("hooked-model"),
        messages: vec![Message {
            role: String::from("user"),
            content: Some(Content::Text(String::from("hi"))),
            name: None,
            provider_specific_fields: JsonObject::default(),
        }],
        non_default_params: JsonObject::default(),
    };
    fixture.prompt.outcome = Some(outcome);
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.model, "hooked-model");
    assert_eq!(lookups(&fixture.metadata)[0].model, "hooked-model");
}

#[tokio::test]
async fn order_cache_seed_reaches_the_prompt_hook() {
    let mut fixture = Fixture::new();
    fixture.prompt.seed_value = Some(json("\"auto\""));
    run(&fixture).await.unwrap();
    let seen = hook_parameters(&fixture.prompt);
    assert!(seen[0].contains_key("cache_control_injection_points"));
}

#[tokio::test]
async fn order_environment_update_runs_after_parameter_mapping() {
    let mut fixture = Fixture::new();
    fixture.args.user = Some(String::from("caller"));
    run(&fixture).await.unwrap();
    let events = fixture.events();
    let mapping = events.iter().position(|event| event == "mapping").unwrap();
    let log = events.iter().position(|event| event == "log").unwrap();
    assert!(mapping < log);
}

#[tokio::test]
async fn model_missing_fails_with_litellm_message() {
    let mut fixture = Fixture::new();
    fixture.args.model = None;
    let error = run(&fixture).await.unwrap_err();
    assert!(matches!(error, CompletionRequestError::ModelNotPassed));
    assert_eq!(error.to_string(), "model param not passed in.");
}

#[tokio::test]
async fn model_messages_are_validated_before_use() {
    let mut fixture = Fixture::new();
    fixture.args.messages = vec![json(r#"{"content":"no role"}"#)];
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.messages[0].role, "assistant");
}

#[tokio::test]
async fn model_stop_is_truncated_to_four_sequences() {
    let mut fixture = Fixture::new();
    fixture.args.stop = Some(json(r#"["a","b","c","d","e"]"#));
    run(&fixture).await.unwrap();
    let recorded = parameters(&fixture.metadata);
    let expected = Stop::Many(
        ["a", "b", "c", "d"]
            .iter()
            .map(|text| (*text).to_owned())
            .collect(),
    );
    assert_eq!(recorded[0].stop, Some(expected));
}

#[tokio::test]
async fn model_tool_choice_collapses_to_auto() {
    let mut fixture = Fixture::new();
    fixture.args.tool_choice = Some(json(r#"{"type":"auto"}"#));
    run(&fixture).await.unwrap();
    let recorded = parameters(&fixture.metadata);
    assert_eq!(
        recorded[0].passthrough.get("tool_choice"),
        Some(&serde_json::json!("auto"))
    );
}

#[tokio::test]
async fn early_exit_fallbacks_carry_the_validated_request() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("fallbacks"), json(r#"["gpt-3.5-turbo"]"#));
    fixture
        .args
        .kwargs
        .insert(String::from("_skip_mcp_handler"), Json::Bool(true));
    fixture.args.messages = vec![json(r#"{"content":"no role"}"#)];
    let outcome = run(&fixture).await.unwrap();
    let PreparedCompletion::Fallbacks(call) = outcome else {
        panic!("expected fallbacks");
    };
    assert_eq!(call.fallbacks.len(), 1);
    assert_eq!(call.args.model.as_deref(), Some("gpt-4o"));
    assert!(!call.args.kwargs.contains_key("_skip_mcp_handler"));
    assert!(!call.args.kwargs.contains_key("_skip_responses_api_bridge"));
    let Some(Json::Object(first)) = call.args.messages.first() else {
        panic!("expected a message object");
    };
    assert_eq!(
        first.get("role"),
        Some(&Json::Text(String::from("assistant")))
    );
    assert!(
        !fixture
            .events()
            .iter()
            .any(|event| event.starts_with("resolve:"))
    );
}

#[tokio::test]
async fn early_exit_model_list_selects_matching_deployments() {
    let mut fixture = Fixture::new();
    let mut matching = JsonObject::new();
    matching.insert(
        String::from("model_name"),
        Json::Text(String::from("gpt-4o")),
    );
    let mut params = JsonObject::new();
    params.insert(
        String::from("api_base"),
        Json::Text(String::from("https://a")),
    );
    matching.insert(String::from("litellm_params"), Json::Object(params));
    let mut other = JsonObject::new();
    other.insert(
        String::from("model_name"),
        Json::Text(String::from("claude")),
    );
    other.insert(
        String::from("litellm_params"),
        Json::Object(JsonObject::new()),
    );
    fixture.args.model_list = Some(vec![matching, other]);
    let outcome = run(&fixture).await.unwrap();
    let PreparedCompletion::ModelList(call) = outcome else {
        panic!("expected a model list");
    };
    assert_eq!(call.deployments.len(), 1);
    assert_eq!(
        call.deployments[0].get("api_base"),
        Some(&Json::Text(String::from("https://a")))
    );
}

#[tokio::test]
async fn early_exit_settings_fallbacks_apply_when_absent() {
    let mut fixture = Fixture::new();
    fixture.settings.model_fallbacks = Some(vec![json("\"claude-3\"")]);
    let outcome = run(&fixture).await.unwrap();
    let PreparedCompletion::Fallbacks(call) = outcome else {
        panic!("expected fallbacks");
    };
    assert_eq!(call.fallbacks, vec![json("\"claude-3\"")]);
}

#[tokio::test]
async fn provider_alias_map_applies_before_resolution() {
    let mut fixture = Fixture::new();
    fixture
        .settings
        .model_alias_map
        .insert(String::from("gpt-4o"), String::from("gpt-4o-mini"));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.model, "gpt-4o-mini");
    assert_eq!(lookups(&fixture.metadata)[0].model, "gpt-4o-mini");
}

#[tokio::test]
async fn provider_azure_flag_pins_the_provider() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("azure"), Json::Bool(true));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.custom_llm_provider.as_deref(), Some("azure"));
    assert_eq!(
        lookups(&fixture.metadata)[0].provider.as_deref(),
        Some("azure")
    );
}

#[tokio::test]
async fn provider_deployment_id_becomes_an_azure_model() {
    let mut fixture = Fixture::new();
    fixture.args.deployment_id = Some(String::from("deployment-1"));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.model, "deployment-1");
    assert_eq!(call.custom_llm_provider.as_deref(), Some("azure"));
}

#[tokio::test]
async fn provider_explicit_settings_base_and_key_win() {
    let mut fixture = Fixture::new();
    fixture.settings.api_base = Some(String::from("https://proxy.test/v1"));
    fixture.settings.api_key = Some(SecretString::from(String::from("global-key")));
    run(&fixture).await.unwrap();
    let recorded = &lookups(&fixture.metadata)[0];
    assert_eq!(recorded.api_base.as_deref(), Some("https://proxy.test/v1"));
    assert!(recorded.has_key);
}

#[tokio::test]
async fn provider_developer_role_becomes_system() {
    let mut fixture = Fixture::new();
    fixture.args.messages = vec![message("developer", "rules"), message("user", "hi")];
    fixture.metadata.translates_developer_role = true;
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.messages[0].role, "system");
}

#[tokio::test]
async fn provider_keeps_developer_role_for_o_series() {
    let mut fixture = Fixture::new();
    fixture.args.messages = vec![message("developer", "rules"), message("user", "hi")];
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.messages[0].role, "developer");
}

#[tokio::test]
async fn provider_supports_system_message_false_maps_system() {
    let mut fixture = Fixture::new();
    fixture.args.messages = vec![message("system", "rules"), message("user", "hi")];
    fixture
        .args
        .kwargs
        .insert(String::from("supports_system_message"), Json::Bool(false));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.messages.len(), 1);
    assert_eq!(call.messages[0].role, "user");
    let Some(Content::Text(text)) = &call.messages[0].content else {
        panic!("expected text content");
    };
    assert_eq!(text, "rules hi");
}

#[tokio::test]
async fn provider_dynamic_key_wins_over_the_request_key() {
    let mut fixture = Fixture::new();
    fixture.args.api_key = Some(SecretString::from(String::from("request-key")));
    fixture.metadata.dynamic_key = Some(String::from("dynamic-key"));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.api_key.unwrap().expose_secret(), "dynamic-key");
}

#[tokio::test]
async fn provider_tool_keys_dropped_for_non_claude_targets() {
    let mut fixture = Fixture::new();
    fixture.args.tools = Some(vec![json(
        r#"{"type":"function","function":{"name":"f"},"input_examples":[]}"#,
    )]);
    let call = call_of(run(&fixture).await.unwrap());
    let Some(Json::Object(tool)) = call.tools.as_ref().and_then(|tools| tools.first()) else {
        panic!("expected a tool object");
    };
    assert!(!tool.contains_key("input_examples"));
}

#[tokio::test]
async fn provider_tool_keys_kept_for_claude_targets() {
    let mut fixture = Fixture::new();
    fixture.metadata.resolved_provider = Some("anthropic");
    fixture.args.tools = Some(vec![json(
        r#"{"type":"function","function":{"name":"f"},"input_examples":[]}"#,
    )]);
    let call = call_of(run(&fixture).await.unwrap());
    let Some(Json::Object(tool)) = call.tools.as_ref().and_then(|tools| tools.first()) else {
        panic!("expected a tool object");
    };
    assert!(tool.contains_key("input_examples"));
}

#[tokio::test]
async fn headers_extra_and_proxy_merge_in_litellm_order() {
    let mut fixture = Fixture::new();
    let mut extra = AdditionalHeaders::new();
    extra.insert(String::from("a"), Json::Text(String::from("extra")));
    extra.insert(String::from("b"), Json::Text(String::from("2")));
    fixture.args.extra_headers = Some(extra);
    let mut headers = JsonObject::new();
    headers.insert(String::from("a"), Json::Text(String::from("1")));
    headers.insert(String::from("h"), Json::Text(String::from("x")));
    fixture
        .args
        .kwargs
        .insert(String::from("headers"), Json::Object(headers));
    let mut proxy = AdditionalHeaders::new();
    proxy.insert(String::from("a"), Json::Text(String::from("proxy")));
    proxy.insert(String::from("c"), Json::Text(String::from("3")));
    fixture.settings.proxy_headers = proxy;
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(header_of(&call, "a").as_deref(), Some("proxy"));
    assert_eq!(header_of(&call, "b").as_deref(), Some("2"));
    assert_eq!(header_of(&call, "c").as_deref(), Some("3"));
    assert_eq!(header_of(&call, "h").as_deref(), Some("x"));
}

#[tokio::test]
async fn headers_provider_specific_applies_after_the_merge() {
    let mut fixture = Fixture::new();
    let mut proxy = AdditionalHeaders::new();
    proxy.insert(String::from("a"), Json::Text(String::from("proxy")));
    fixture.settings.proxy_headers = proxy;
    fixture.args.kwargs.insert(
        String::from("provider_specific_header"),
        json(r#"{"custom_llm_provider":"openai","extra_headers":{"a":"specific"}}"#),
    );
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(header_of(&call, "a").as_deref(), Some("specific"));
}

#[tokio::test]
async fn headers_affinity_uses_the_request_session() {
    let mut fixture = Fixture::new();
    fixture.settings.optional.provider_affinity_header = Some(String::from("x-session"));
    fixture.args.kwargs.insert(
        String::from("session_id"),
        Json::Text(String::from("session-1")),
    );
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(header_of(&call, "x-session").as_deref(), Some("session-1"));
}

#[tokio::test]
async fn headers_affinity_forbidden_name_is_a_bad_request() {
    let mut fixture = Fixture::new();
    fixture.settings.optional.provider_affinity_header = Some(String::from("authorization"));
    let error = run(&fixture).await.unwrap_err();
    assert!(matches!(error, CompletionRequestError::Affinity(_)));
}

#[tokio::test]
async fn bridge_early_check_keeps_plain_chat_on_chat() {
    let mut fixture = Fixture::new();
    fixture.args.reasoning_effort = Some(json("\"high\""));
    fixture.args.tools = Some(vec![json(r#"{"type":"function","function":{"name":"f"}}"#)]);
    fixture.bridge.gpt5 = Some(true);
    let outcome = run(&fixture).await.unwrap();
    assert!(matches!(outcome, PreparedCompletion::Dispatch(_)));
    let checks = fixture
        .events()
        .iter()
        .filter(|event| *event == "bridge_info")
        .count();
    assert_eq!(checks, 2);
}

#[tokio::test]
async fn bridge_gpt5_tools_bridge_at_the_second_check() {
    let mut fixture = Fixture::new();
    fixture.args.reasoning_effort = Some(json("\"high\""));
    fixture.args.tools = Some(vec![json(r#"{"type":"function","function":{"name":"f"}}"#)]);
    fixture.bridge.gpt5 = Some(true);
    fixture.bridge.gpt5_4 = Some(true);
    let outcome = run(&fixture).await.unwrap();
    assert!(matches!(outcome, PreparedCompletion::ResponsesBridge(_)));
}

#[tokio::test]
async fn bridge_skip_flag_keeps_the_call_on_chat() {
    let mut fixture = Fixture::new();
    fixture.args.reasoning_effort = Some(json("\"high\""));
    fixture.args.tools = Some(vec![json(r#"{"type":"function","function":{"name":"f"}}"#)]);
    fixture.bridge.gpt5 = Some(true);
    fixture.bridge.gpt5_4 = Some(true);
    fixture
        .args
        .kwargs
        .insert(String::from("_skip_responses_api_bridge"), Json::Bool(true));
    let outcome = run(&fixture).await.unwrap();
    assert!(matches!(outcome, PreparedCompletion::Dispatch(_)));
}

#[tokio::test]
async fn bridge_reasoning_summary_alias_merges_into_effort() {
    let mut fixture = Fixture::new();
    fixture.args.reasoning_effort = Some(json("\"high\""));
    fixture.metadata.mapping = Some(mapping_with(
        &[],
        &[("reasoning_summary", json("\"auto\""))],
    ));
    fixture.bridge.gpt5 = Some(true);
    let outcome = run(&fixture).await.unwrap();
    let PreparedCompletion::ResponsesBridge(call) = outcome else {
        panic!("expected the responses bridge");
    };
    let effort = &call.options.params["reasoning_effort"];
    assert_eq!(
        effort,
        &convert_value(&json(r#"{"effort":"high","summary":"auto"}"#))
    );
}

#[tokio::test]
async fn bridge_object_effort_with_summary_is_kept() {
    let mut fixture = Fixture::new();
    fixture.args.reasoning_effort = Some(json(r#"{"effort":"high","summary":"auto"}"#));
    fixture.settings.route_all_to_responses = true;
    let outcome = run(&fixture).await.unwrap();
    let PreparedCompletion::ResponsesBridge(call) = outcome else {
        panic!("expected the responses bridge");
    };
    assert_eq!(
        &call.options.params["reasoning_effort"],
        &convert_value(&json(r#"{"effort":"high","summary":"auto"}"#))
    );
}

#[tokio::test]
async fn bridge_gpt5_chat_loses_the_aliases() {
    let mut fixture = Fixture::new();
    fixture.metadata.mapping = Some(mapping_with(&[], &[("reasoningSummary", json("\"auto\""))]));
    fixture.bridge.gpt5 = Some(true);
    let call = call_of(run(&fixture).await.unwrap());
    assert!(!call.options.params.contains_key("reasoningSummary"));
    assert!(!call.options.params.contains_key("reasoning_summary"));
}

#[rstest::rstest]
#[case::proxy_tool("litellm_proxy", true)]
#[case::proxy_path("https://gateway.test/mcp/tools", true)]
#[case::other_url("https://gateway.test/v1", false)]
#[tokio::test]
async fn bridge_mcp_tools_route_to_the_loop(#[case] server_url: &str, #[case] expected: bool) {
    let mut fixture = Fixture::new();
    fixture.args.tools = Some(vec![json(&format!(
        r#"{{"type":"mcp","server_url":"{server_url}"}}"#
    ))]);
    let outcome = run(&fixture).await.unwrap();
    assert_eq!(matches!(outcome, PreparedCompletion::McpLoop(_)), expected);
}

#[rstest::rstest]
#[case::both_token_prices(r#"{"input_cost_per_token":0.1,"output_cost_per_token":0.2}"#, true)]
#[case::input_only(r#"{"input_cost_per_token":0.1}"#, false)]
#[case::per_second(r#"{"cost_per_second":0.1}"#, true)]
#[case::absent("{}", false)]
#[tokio::test]
async fn pricing_registers_only_when_the_gate_holds(#[case] extra: &str, #[case] expected: bool) {
    let mut fixture = Fixture::new();
    let Json::Object(costs) = json(extra) else {
        panic!("expected an object");
    };
    fixture.args.kwargs.extend(costs);
    run(&fixture).await.unwrap();
    assert_eq!(
        !fixture.pricing.requests.lock().unwrap().is_empty(),
        expected
    );
}

#[tokio::test]
async fn pricing_sees_the_resolved_model_and_provider() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("input_cost_per_token"), json("0.1"));
    fixture
        .args
        .kwargs
        .insert(String::from("output_cost_per_token"), json("0.2"));
    fixture.metadata.resolved_model = Some(String::from("gpt-4o-2024"));
    run(&fixture).await.unwrap();
    let requests = fixture.pricing.requests.lock().unwrap();
    assert_eq!(requests[0].0, "gpt-4o-2024");
    assert_eq!(requests[0].1, "openai");
}

#[tokio::test]
async fn mock_request_gives_the_mock_outcome() {
    let mut fixture = Fixture::new();
    fixture.args.kwargs.insert(
        String::from("mock_response"),
        Json::Text(String::from("hello")),
    );
    fixture.args.kwargs.insert(
        String::from("litellm_metadata"),
        json(r#"{"user_api_key_budget_reservation":{"input_tokens":12}}"#),
    );
    let outcome = run(&fixture).await.unwrap();
    let PreparedCompletion::Mock(call) = outcome else {
        panic!("expected the mock outcome");
    };
    assert_eq!(call.call.model, "gpt-4o");
    assert_eq!(call.prompt_tokens, Some(12));
    assert!(!call.acompletion);
}

#[tokio::test]
async fn mock_empty_response_stays_on_dispatch() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("mock_response"), Json::Text(String::new()));
    let outcome = run(&fixture).await.unwrap();
    assert!(matches!(outcome, PreparedCompletion::Dispatch(_)));
}

#[tokio::test]
async fn mock_timeout_alone_returns_the_mock_outcome() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("mock_timeout"), Json::Bool(true));
    let outcome = run(&fixture).await.unwrap();
    assert!(matches!(outcome, PreparedCompletion::Mock(_)));
}

#[tokio::test]
async fn async_empty_tools_become_none_after_the_hook() {
    let mut fixture = Fixture::new();
    fixture.args.tools = Some(Vec::new());
    fixture.prompt.should_run = true;
    let call = call_of(run_async(&fixture).await.unwrap());
    assert!(call.tools.is_none());
}

#[tokio::test]
async fn async_global_fallbacks_apply() {
    let mut fixture = Fixture::new();
    fixture.settings.model_fallbacks = Some(vec![json("\"claude-3\"")]);
    let outcome = run_async(&fixture).await.unwrap();
    assert!(matches!(outcome, PreparedCompletion::Fallbacks(_)));
}

#[tokio::test(start_paused = true)]
async fn async_mock_timeout_sleeps_then_fails() {
    let mut fixture = Fixture::new();
    fixture.args.timeout = Some(json("10"));
    fixture
        .args
        .kwargs
        .insert(String::from("mock_timeout"), Json::Bool(true));
    let error = run_async(&fixture).await.unwrap_err();
    assert!(matches!(error, CompletionRequestError::MockTimeout));
    assert_eq!(error.to_string(), "This is a mock timeout error");
}

#[tokio::test]
async fn async_hook_uses_the_async_form() {
    let mut fixture = Fixture::new();
    fixture.prompt.should_run = true;
    run_async(&fixture).await.unwrap();
    assert!(fixture.events().iter().any(|event| event == "hook_async"));
}

#[tokio::test]
async fn async_resolves_the_provider_when_the_caller_gave_none() {
    let fixture = Fixture::new();
    let call = call_of(run_async(&fixture).await.unwrap());
    assert_eq!(call.custom_llm_provider.as_deref(), Some("openai"));
    let lookups = lookups(&fixture.metadata);
    assert_eq!(lookups[0].provider, None);
}

#[tokio::test]
async fn router_call_pins_retries_to_zero() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("num_retries"), json("5"));
    fixture
        .args
        .kwargs
        .insert(String::from("metadata"), json(r#"{"model_group":"chat"}"#));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.max_retries, Some(0));
}

#[tokio::test]
async fn router_direct_call_keeps_num_retries() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("num_retries"), json("5"));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.max_retries, Some(5));
}

#[tokio::test]
async fn router_direct_call_keeps_max_retries() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("max_retries"), json("2"));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.max_retries, Some(2));
}

#[tokio::test]
async fn provider_environment_update_records_the_call() {
    let mut fixture = Fixture::new();
    fixture.args.user = Some(String::from("caller"));
    run(&fixture).await.unwrap();
    let updates = fixture.logging.updates.lock().unwrap();
    assert_eq!(updates[0].model, "gpt-4o");
    assert_eq!(updates[0].user.as_deref(), Some("caller"));
    assert_eq!(updates[0].provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn provider_timeout_phases_collapse_for_a_provider_without_support() {
    let mut fixture = Fixture::new();
    fixture.metadata.resolved_provider = Some("anthropic");
    fixture.args.timeout = Some(json(r#"{"connect":1,"read":7}"#));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(call.timeout, Some(TimeoutValue::Seconds(Seconds(7.0))));
}

#[tokio::test]
async fn provider_base_url_becomes_the_api_base() {
    let mut fixture = Fixture::new();
    fixture.args.base_url = Some(String::from("https://base.test"));
    fixture
        .args
        .kwargs
        .insert(String::from("api_base"), json("\"https://kw.test\""));
    run(&fixture).await.unwrap();
    assert_eq!(
        lookups(&fixture.metadata)[0].api_base.as_deref(),
        Some("https://base.test")
    );
}

#[tokio::test]
async fn provider_hidden_params_record_provider_and_region() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("aws_region_name"), json("\"us-east-1\""));
    let call = call_of(run(&fixture).await.unwrap());
    assert_eq!(
        call.hidden_params.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(
        call.hidden_params.get("region_name"),
        Some(&Json::Text(String::from("us-east-1")))
    );
}

#[tokio::test]
async fn provider_custom_prompt_dict_collects_template_keys() {
    let mut fixture = Fixture::new();
    fixture
        .args
        .kwargs
        .insert(String::from("initial_prompt_value"), json("\"hi\""));
    fixture
        .args
        .kwargs
        .insert(String::from("bos_token"), json("\"<s>\""));
    let call = call_of(run(&fixture).await.unwrap());
    let Some(Json::Object(template)) = call.custom_prompt_dict.get("gpt-4o") else {
        panic!("expected a custom prompt template");
    };
    assert_eq!(
        template.get("initial_prompt_value"),
        Some(&Json::Text(String::from("hi")))
    );
    assert_eq!(
        template.get("bos_token"),
        Some(&Json::Text(String::from("<s>")))
    );
}

#[tokio::test]
async fn provider_file_ids_use_the_model_mapping() {
    let mut fixture = Fixture::new();
    fixture.args.messages = vec![json(
        r#"{"role":"user","content":[{"type":"file","file":{"file_id":"file-1"}}]}"#,
    )];
    fixture
        .args
        .kwargs
        .insert(String::from("model_info"), json(r#"{"id":"model-1"}"#));
    fixture.args.kwargs.insert(
        String::from("model_file_id_mapping"),
        json(r#"{"file-1":{"model-1":"provider-file"}}"#),
    );
    let call = call_of(run(&fixture).await.unwrap());
    let messages: Vec<Json> = convert(&call.messages).unwrap();
    let Some(Json::Object(message)) = messages.first() else {
        panic!("expected a message object");
    };
    let Some(Json::List(parts)) = message.get("content") else {
        panic!("expected content parts");
    };
    let Some(Json::Object(part)) = parts.first() else {
        panic!("expected a part object");
    };
    let Some(Json::Object(file)) = part.get("file") else {
        panic!("expected a file object");
    };
    assert_eq!(
        file.get("file_id"),
        Some(&Json::Text(String::from("provider-file")))
    );
}

#[tokio::test]
async fn provider_system_prompt_merges_into_the_first_system_message() {
    let mut fixture = Fixture::new();
    fixture.args.messages = vec![message("system", "base"), message("user", "hi")];
    fixture
        .args
        .kwargs
        .insert(String::from("litellm_system_prompt"), json("\"extra\""));
    let call = call_of(run(&fixture).await.unwrap());
    let Some(Content::Text(text)) = &call.messages[0].content else {
        panic!("expected text content");
    };
    assert_eq!(text, "extra\n\nbase");
}

#[tokio::test]
async fn provider_environment_uses_the_processed_parameters() {
    let mut fixture = Fixture::new();
    fixture.args.max_tokens = Some(5);
    fixture
        .args
        .kwargs
        .insert(String::from("custom_key"), json("\"secret\""));
    run(&fixture).await.unwrap();
    let updates = fixture.logging.updates.lock().unwrap().clone();
    assert!(updates[0].optional_params.contains_key("max_tokens"));
    assert!(!updates[0].optional_params.contains_key("custom_key"));
}

#[tokio::test]
async fn provider_ollama_function_calling_falls_back_to_prompt() {
    let mut fixture = Fixture::new();
    fixture.metadata.resolved_provider = Some("ollama");
    fixture.args.tools = Some(vec![json(r#"{"type":"function","function":{"name":"f"}}"#)]);
    let call = call_of(run(&fixture).await.unwrap());
    let system = call
        .messages
        .iter()
        .find(|message| message.role == "system");
    let Some(Content::Text(text)) = system.and_then(|message| message.content.as_ref()) else {
        panic!("expected a system prompt with text");
    };
    assert!(text.contains("Produce JSON OUTPUT ONLY"));
}

#[tokio::test]
async fn bridge_allowed_params_gain_reasoning_effort() {
    let mut fixture = Fixture::new();
    fixture.settings.route_all_to_responses = true;
    run(&fixture).await.unwrap();
    let policies = fixture.metadata.policies.lock().unwrap().clone();
    assert!(
        policies[0]
            .allowed_openai_params
            .contains(&String::from("reasoning_effort"))
    );
}

#[tokio::test]
async fn bridge_allowed_params_stay_plain_without_the_bridge() {
    let fixture = Fixture::new();
    run(&fixture).await.unwrap();
    let policies = fixture.metadata.policies.lock().unwrap().clone();
    assert!(policies[0].allowed_openai_params.is_empty());
}

fn convert<Out, In>(value: &In) -> super::error::RequestResult<Out>
where
    In: serde::Serialize + ?Sized,
    Out: DeserializeOwned,
{
    super::steps::convert::convert(value)
}

fn convert_value(value: &Json) -> JsonValue {
    super::steps::convert::convert(value).unwrap()
}
