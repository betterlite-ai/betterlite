#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use serde::Serialize;
use serde_json::{Map, Value};

use super::{
    ChatBase, DUMMY_TOOLS, HiddenCarrier, HistoryMessage, ImageEditBase, ResponseKind,
    ResponsesBase, add_thinking_max_tokens, append_tools, assistant_has_thinking, has_tool_calls,
    hidden_params, last_tool_call_unthought, provider_config, response_format_refs,
    thinking_request,
};
use crate::error::{ErrorRequest, ErrorResponse, OpenAiError, ProviderError, ProviderErrorParts};

fn object(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).unwrap()
}

fn history(text: &str) -> Vec<HistoryMessage> {
    serde_json::from_str(text).unwrap()
}

fn load_settings(source: &str, environment: &[(&str, &str)]) -> (bool, i64) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let settings = ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .litellm_settings;
    (settings.modify_params, settings.default_max_tokens)
}

fn base_error_projection() {
    let parts: ProviderErrorParts = serde_json::from_str(
        r#"{"status_code": 429, "message": "slow down", "status_code_is_synthesized": false}"#,
    )
    .unwrap();
    let synthesized = ProviderError::from(parts.clone());
    let docs = ErrorRequest {
        method: "POST".to_owned(),
        url: "https://docs.litellm.ai/docs".to_owned(),
    };
    let response = ErrorResponse {
        status_code: 429,
        request: docs.clone(),
    };
    assert_eq!(
        (&synthesized.request, &synthesized.response),
        (&docs, &response)
    );
    assert_eq!((&synthesized.headers, &synthesized.body), (&None, &None));
    assert!(!synthesized.status_code_is_synthesized);
    let message = OpenAiError::Provider(Box::new(synthesized)).to_string();
    assert_eq!(message, "slow down");
    let supplied = ProviderError::from(ProviderErrorParts {
        headers: Some(vec![("retry-after".to_owned(), "1".to_owned())]),
        request: Some(response.request),
        response: Some(ErrorResponse {
            status_code: 503,
            request: docs,
        }),
        body: Some(object(r#"{"error": {"code": "rate"}}"#)),
        status_code_is_synthesized: true,
        ..parts
    });
    let expected = r#"{"status_code": 429, "status_code_is_synthesized": true, "message": "slow down",
        "headers": [["retry-after", "1"]], "body": {"error": {"code": "rate"}},
        "request": {"method": "POST", "url": "https://docs.litellm.ai/docs"},
        "response": {"status_code": 503, "request": {"method": "POST", "url": "https://docs.litellm.ai/docs"}}}"#;
    let observed = serde_json::to_value(supplied).unwrap();
    assert_eq!(observed, Value::Object(object(expected)));
}

#[derive(Serialize)]
struct FixtureConfig {
    max_tokens: Option<u32>,
    stream: Option<bool>,
    stop: Option<Vec<String>>,
    top_k: Option<u32>,
    #[serde(rename = "_private")]
    private: Option<u32>,
}

fn class_config_filter() {
    let config = FixtureConfig {
        max_tokens: Some(0),
        stream: Some(false),
        stop: Some(Vec::new()),
        top_k: None,
        private: Some(1),
    };
    let expected = object(r#"{"max_tokens": 0, "stream": false, "stop": []}"#);
    assert_eq!(provider_config(&config).unwrap(), expected);
    assert_eq!(provider_config(&ChatBase).unwrap(), Map::new());
    let scalar = provider_config(&"text");
    assert!(matches!(scalar, Err(OpenAiError::ConfigNotObject)));
}

fn thinking_option_presence() {
    let enabled = |text: &str| thinking_request(&object(text)).thinking;
    let max_tokens = |text: &str| thinking_request(&object(text)).max_tokens;
    assert!(enabled(r#"{"thinking": true}"#));
    assert!(enabled(r#"{"thinking": {"type": "enabled"}}"#));
    assert!(enabled(r#"{"reasoning_effort": "none"}"#));
    assert!(enabled(r#"{"reasoning_effort": ""}"#));
    assert!(enabled(r#"{"reasoning_effort": false}"#));
    assert!(!enabled(r#"{"thinking": {"type": "adaptive"}}"#));
    assert!(!enabled(
        r#"{"thinking": 1, "reasoning_effort": null, "max_tokens": 2}"#
    ));
    assert!(!enabled(r#"{"thinking": "enabled"}"#));
    assert!(!max_tokens(r#"{"thinking": true}"#));
    assert!(max_tokens(r#"{"max_tokens": null}"#));
    assert!(max_tokens(r#"{"max_completion_tokens": 3}"#));
}

fn implicit_thinking_max_tokens() {
    let budget = r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 9}"#;
    let expected =
        r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 5120}"#;
    assert_eq!(add_tokens("{}", budget).unwrap(), object(expected));
    let (_, from_config) = load_settings("", &[("DEFAULT_MAX_TOKENS", " 100 ")]);
    let mut configured = object(budget);
    add_thinking_max_tokens(&Map::new(), &mut configured, from_config).unwrap();
    assert_eq!(configured["max_tokens"], 1124_i64);
    assert_eq!(load_settings("", &[]).1, 4096);
    let text = add_tokens(
        "{}",
        r#"{"thinking": {"type": "enabled", "budget_tokens": "5"}}"#,
    );
    assert!(matches!(text, Err(OpenAiError::BudgetNotInteger)));
    let huge = r#"{"thinking": {"type": "enabled", "budget_tokens": 9223372036854775807}}"#;
    assert!(matches!(
        add_tokens("{}", huge),
        Err(OpenAiError::BudgetOverflow)
    ));
}

fn add_tokens(requested: &str, options: &str) -> Result<Map<String, Value>, OpenAiError> {
    let mut updated = object(options);
    add_thinking_max_tokens(&object(requested), &mut updated, 4096).map(|()| updated)
}

fn implicit_thinking_left_alone() {
    let budget = r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 9}"#;
    let unchanged =
        |requested: &str, options: &str| add_tokens(requested, options).unwrap() == object(options);
    assert!(unchanged(r#"{"max_tokens": null}"#, budget));
    assert!(unchanged(r#"{"max_completion_tokens": 1}"#, budget));
    assert!(unchanged("{}", r#"{"thinking": {"type": "enabled"}}"#));
    assert!(unchanged(
        "{}",
        r#"{"thinking": {"type": "enabled", "budget_tokens": null}}"#
    ));
    assert!(unchanged(
        "{}",
        r#"{"reasoning_effort": "low", "thinking": [1]}"#
    ));
    assert!(unchanged(
        "{}",
        r#"{"thinking": {"type": "adaptive", "budget_tokens": 5}}"#
    ));
}

fn append_tools_cases() {
    let tools = |text: &str| serde_json::from_str::<Vec<Value>>(text).unwrap();
    let mut options = object(r#"{"stream": true}"#);
    append_tools(&mut options, tools(r#"[{"name": "a"}]"#)).unwrap();
    append_tools(&mut options, tools(r#"[{"name": "a"}, {"name": "b"}]"#)).unwrap();
    let expected =
        object(r#"{"stream": true, "tools": [{"name": "a"}, {"name": "a"}, {"name": "b"}]}"#);
    assert_eq!(options, expected);
    let mut nulled = object(r#"{"tools": null}"#);
    let failed = append_tools(&mut nulled, tools("[{}]"));
    assert!(matches!(failed, Err(OpenAiError::ToolsNotList)));
    assert_eq!(nulled, object(r#"{"tools": null}"#));
}

fn refs(format: &str, template: &str) -> Result<Option<Map<String, Value>>, OpenAiError> {
    response_format_refs(Some(&object(format)), Some(template))
}

fn schema(inner: &str) -> String {
    format!(r#"{{"type": "json_schema", "json_schema": {{"schema": {inner}}}}}"#)
}

fn dict_response_schema_refs() {
    assert_eq!(response_format_refs(None, Some("{model}")).unwrap(), None);
    let plain = object(r#"{"type": "json_schema", "json_schema": 3}"#);
    assert_eq!(
        response_format_refs(Some(&plain), None).unwrap(),
        Some(plain)
    );
    let other = r#"{"type": "json_object", "json_schema": 3}"#;
    assert_eq!(refs(other, "{x}").unwrap(), Some(object(other)));
    let format = r##"{"type": "json_schema", "extra": 1, "json_schema": {"name": "n", "schema": {"$ref": "#/$defs/Root",
        "$defs": {"Root": {"anyOf": [{"$ref": "Leaf"}, [{"$ref": "a/b/Item"}]], "default": {"$ref": "x/"}}}}}}"##;
    let expected = r#"{"type": "json_schema", "extra": 1, "json_schema": {"name": "n", "schema": {"$ref": "{m}/Root!",
        "$defs": {"Root": {"anyOf": [{"$ref": "{m}/Leaf!"}, [{"$ref": "{m}/Item!"}]], "default": {"$ref": "{m}/!"}}}}}}"#;
    let input = object(format);
    let rewritten = response_format_refs(Some(&input), Some("{{m}}/{model}!")).unwrap();
    assert_eq!((rewritten, input), (Some(object(expected)), object(format)));
    assert_eq!(
        refs(&schema("[1]"), "{bad").unwrap(),
        Some(object(&schema("[1]")))
    );
}

fn dict_response_schema_errors() {
    let failure =
        |format: &str, template: &str| format!("{:?}", refs(format, template).unwrap_err());
    let missing = failure(r#"{"type": "json_schema"}"#, "{model}");
    assert_eq!(missing, r#"MissingKey { key: "json_schema" }"#);
    let text = failure(r#"{"type": "json_schema", "json_schema": "s"}"#, "{model}");
    assert_eq!(text, r#"NotSubscriptable { key: "json_schema" }"#);
    let no_schema = failure(r#"{"type": "json_schema", "json_schema": {}}"#, "{model}");
    assert_eq!(no_schema, r#"MissingKey { key: "schema" }"#);
    assert_eq!(
        failure(&schema(r#"[{"$ref": 1}]"#), "{model}"),
        "RefNotText"
    );
    let reference = schema(r#"{"$ref": "a"}"#);
    assert_eq!(
        failure(&reference, "{name}"),
        r#"TemplateField { field: "name" }"#
    );
    assert_eq!(failure(&reference, "{0}"), "TemplatePosition");
    assert_eq!(failure(&reference, "{}"), "TemplatePosition");
    assert_eq!(failure(&reference, "x}"), "TemplateBrace");
    assert_eq!(failure(&reference, "{model"), "TemplateBrace");
}

fn hidden_metadata_projection() {
    let carrier = |kind: ResponseKind, hidden: Option<&str>| HiddenCarrier {
        kind,
        hidden_params: hidden.map(object),
    };
    let mut stamped = carrier(ResponseKind::Dict, Some(r#"{"model_id": "m"}"#));
    assert_eq!(
        hidden_params(&mut stamped, true),
        object(r#"{"model_id": "m"}"#)
    );
    assert_eq!(
        stamped,
        carrier(ResponseKind::Dict, Some(r#"{"model_id": "m"}"#))
    );
    let mut missing = carrier(ResponseKind::Dict, None);
    assert_eq!(hidden_params(&mut missing, false), Map::new());
    assert_eq!(missing.hidden_params, None);
    assert_eq!(hidden_params(&mut missing, true), Map::new());
    assert_eq!(missing.hidden_params, Some(Map::new()));
    let mut object_response = carrier(ResponseKind::Object, None);
    assert_eq!(hidden_params(&mut object_response, true), Map::new());
    assert_eq!(object_response.hidden_params, None);
}

fn history_tool_and_thinking_predicates() {
    let calls = |text: &str| has_tool_calls(&history(text));
    assert!(calls(
        r#"[{"role": "assistant", "tool_calls": null}, {"role": "user", "tool_calls": []}]"#
    ));
    assert!(!calls(r#"[{"role": "assistant", "tool_calls": null}]"#));
    let thinking = |text: &str| assistant_has_thinking(&history(text));
    assert!(thinking(
        r#"[{"role": "assistant", "thinking_blocks": [{"type": "thinking"}]}]"#
    ));
    assert!(thinking(r#"[{"role": "assistant", "thinking_blocks": 5}]"#));
    assert!(thinking(
        r#"[{"role": "assistant", "thinking_blocks": "x"}]"#
    ));
    assert!(!thinking(
        r#"[{"role": "assistant", "thinking_blocks": []}, {"role": "assistant", "thinking_blocks": ""}]"#
    ));
    assert!(!thinking(
        r#"[{"role": "user", "thinking_blocks": [1]}, {"role": "assistant"}]"#
    ));
    let unthought = |text: &str| last_tool_call_unthought(&history(text));
    assert!(!unthought(
        r#"[{"role": "user", "tool_calls": []}, {"role": "assistant"}]"#
    ));
    assert!(unthought(
        r#"[{"role": "assistant", "tool_calls": [], "thinking_blocks": null}]"#
    ));
    assert!(unthought(
        r#"[{"role": "assistant", "tool_calls": [], "thinking_blocks": [1]},
            {"role": "assistant", "tool_calls": [], "thinking_blocks": {}}]"#
    ));
    assert!(!unthought(
        r#"[{"role": "assistant", "tool_calls": [], "thinking_blocks": [1]}]"#
    ));
    assert!(!unthought(
        r#"[{"role": "assistant", "tool_calls": [], "thinking_blocks": 7}]"#
    ));
}

fn dummy_tool_literal() {
    let expected = r#"[{"type": "function", "function": {"name": "dummy_tool",
        "description": "This is a dummy tool call", "parameters": {"type": "object", "properties": {}}}}]"#;
    let observed = serde_json::to_value(DUMMY_TOOLS).unwrap();
    assert_eq!(observed, serde_json::from_str::<Value>(expected).unwrap());
}

fn modify_params_default_and_env() {
    let modify = |source: &str, environment: &[(&str, &str)]| load_settings(source, environment).0;
    assert!(!modify("", &[]));
    assert!(!modify("", &[("LITELLM_MODIFY_PARAMS", "")]));
    assert!(modify("", &[("LITELLM_MODIFY_PARAMS", "false")]));
    assert!(modify("", &[("LITELLM_MODIFY_PARAMS", "0")]));
    assert!(modify("", &[("LITELLM_MODIFY_PARAMS", " ")]));
    let configured_off = "litellm_settings:\n  modify_params: false\n";
    assert!(!modify(
        configured_off,
        &[("LITELLM_MODIFY_PARAMS", "true")]
    ));
    assert!(modify("litellm_settings:\n  modify_params: true\n", &[]));
}

fn base_responses_initializer() {
    assert_eq!(provider_config(&ResponsesBase).unwrap(), Map::new());
}

fn base_image_edit_initializer() {
    assert_eq!(provider_config(&ImageEditBase).unwrap(), Map::new());
}

#[rstest]
#[case::base_error_projection(base_error_projection)]
#[case::class_config_filter(class_config_filter)]
#[case::thinking_option_presence(thinking_option_presence)]
#[case::implicit_thinking_max_tokens(implicit_thinking_max_tokens)]
#[case::implicit_thinking_max_tokens(implicit_thinking_left_alone)]
#[case::append_tools(append_tools_cases)]
#[case::dict_response_schema_refs(dict_response_schema_refs)]
#[case::dict_response_schema_refs(dict_response_schema_errors)]
#[case::hidden_metadata_projection(hidden_metadata_projection)]
#[case::history_tool_and_thinking_predicates(history_tool_and_thinking_predicates)]
#[case::dummy_tool_literal(dummy_tool_literal)]
#[case::modify_params_default_and_env(modify_params_default_and_env)]
#[case::base_responses_initializer(base_responses_initializer)]
#[case::base_image_edit_initializer(base_image_edit_initializer)]
fn base_helpers_cases(#[case] check: fn()) {
    check();
}
