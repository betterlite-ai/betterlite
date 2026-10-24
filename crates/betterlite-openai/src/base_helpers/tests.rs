#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{
    ChatBase, DUMMY_TOOLS, HiddenCarrier, HistoryMessage, ImageEditBase, ResponseKind,
    ResponsesBase, ThinkingRequest, add_thinking_max_tokens, append_tools, assistant_has_thinking,
    has_tool_calls, hidden_params, last_tool_call_unthought, provider_config,
    response_format_refs, thinking_request,
};
use crate::error::{ErrorRequest, ErrorResponse, OpenAiError, ProviderError, ProviderErrorParts};

fn object(value: Value) -> Map<String, Value> {
    serde_json::from_value(value).unwrap()
}

fn history(value: Value) -> Vec<HistoryMessage> {
    serde_json::from_value(value).unwrap()
}

fn load_settings(source: &str, environment: &[(&str, &str)]) -> (bool, i64) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let settings = ConfigLoader { lookup }.load(source).unwrap().litellm_settings;
    (settings.modify_params, settings.default_max_tokens)
}

fn base_error_projection() {
    let parts = ProviderErrorParts {
        status_code: 429,
        message: "slow down".to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    };
    let synthesized = ProviderError::from(parts.clone());
    let docs = ErrorRequest {
        method: "POST".to_owned(),
        url: "https://docs.litellm.ai/docs".to_owned(),
    };
    assert_eq!(synthesized.request, docs);
    assert_eq!(synthesized.response.status_code, 429);
    assert_eq!(synthesized.response.request, docs);
    assert_eq!((synthesized.headers.clone(), synthesized.body.clone()), (None, None));
    assert!(!synthesized.status_code_is_synthesized);
    let message = OpenAiError::Provider(Box::new(synthesized)).to_string();
    assert_eq!(message, "slow down");
    let upstream = ErrorRequest {
        method: "GET".to_owned(),
        url: "https://upstream.test/v1".to_owned(),
    };
    let response = ErrorResponse {
        status_code: 503,
        request: upstream.clone(),
    };
    let supplied = ProviderError::from(ProviderErrorParts {
        headers: Some(vec![("retry-after".to_owned(), "1".to_owned())]),
        request: Some(upstream.clone()),
        response: Some(response.clone()),
        body: Some(object(json!({"error": {"code": "rate"}}))),
        status_code_is_synthesized: true,
        ..parts
    });
    assert_eq!((supplied.request, supplied.response), (upstream, response));
    assert_eq!(supplied.status_code, 429);
    assert!(supplied.status_code_is_synthesized);
    assert_eq!(supplied.body, Some(object(json!({"error": {"code": "rate"}}))));
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
    let expected = object(json!({"max_tokens": 0, "stream": false, "stop": []}));
    assert_eq!(provider_config(&config).unwrap(), expected);
    assert_eq!(provider_config(&ChatBase).unwrap(), Map::new());
    assert!(matches!(provider_config(&3), Err(OpenAiError::ConfigNotObject)));
}

fn thinking_option_presence() {
    let request = |params: Value| thinking_request(&object(params));
    let enabled = |params: Value| request(params).thinking;
    assert!(enabled(json!({"thinking": true})));
    assert!(enabled(json!({"thinking": {"type": "enabled"}})));
    assert!(enabled(json!({"reasoning_effort": "none"})));
    assert!(enabled(json!({"reasoning_effort": ""})));
    assert!(enabled(json!({"reasoning_effort": false})));
    assert!(!enabled(json!({"thinking": {"type": "adaptive"}})));
    assert!(!enabled(json!({"thinking": 1, "reasoning_effort": null})));
    assert!(!enabled(json!({"thinking": "enabled"})));
    let absent = ThinkingRequest {
        thinking: false,
        max_tokens: false,
    };
    assert_eq!(request(json!({})), absent);
    assert!(request(json!({"max_tokens": null})).max_tokens);
    assert!(request(json!({"max_completion_tokens": 3})).max_tokens);
}

fn implicit_thinking_max_tokens() {
    let run = |requested: Value, options: Value, default: i64| {
        let mut updated = object(options);
        add_thinking_max_tokens(&object(requested), &mut updated, default).map(|()| updated)
    };
    let budget = json!({"thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 9});
    let expected = json!({"thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 5120});
    assert_eq!(run(json!({}), budget.clone(), 4096).unwrap(), object(expected));
    let (_, from_config) = load_settings("", &[("DEFAULT_MAX_TOKENS", " 100 ")]);
    assert_eq!(run(json!({}), budget.clone(), from_config).unwrap()["max_tokens"], 1124);
    assert_eq!(load_settings("", &[]).1, 4096);
    let unchanged = [
        (json!({"max_tokens": null}), budget.clone()),
        (json!({"max_completion_tokens": 1}), budget),
        (json!({}), json!({"thinking": {"type": "enabled"}})),
        (json!({}), json!({"thinking": {"type": "enabled", "budget_tokens": null}})),
        (json!({}), json!({"reasoning_effort": "low", "thinking": [1]})),
        (json!({}), json!({"thinking": {"type": "adaptive", "budget_tokens": 5}})),
    ];
    assert!(
        unchanged
            .into_iter()
            .all(|(requested, options)| run(requested, options.clone(), 4096).unwrap() == object(options))
    );
    let text_budget = json!({"thinking": {"type": "enabled", "budget_tokens": "5"}});
    assert!(matches!(run(json!({}), text_budget, 1), Err(OpenAiError::BudgetNotInteger)));
    let huge = json!({"thinking": {"type": "enabled", "budget_tokens": i64::MAX}});
    assert!(matches!(run(json!({}), huge, 1), Err(OpenAiError::BudgetOverflow)));
}

fn append_tools_cases() {
    let first = json!({"type": "function", "function": {"name": "a"}});
    let mut options = object(json!({"stream": true}));
    append_tools(&mut options, vec![first.clone()]).unwrap();
    append_tools(&mut options, vec![first.clone(), json!({"name": "b"})]).unwrap();
    assert_eq!(options["tools"], json!([first, first, {"name": "b"}]));
    let mut nulled = object(json!({"tools": null}));
    let failed = append_tools(&mut nulled, vec![json!({})]);
    assert!(matches!(failed, Err(OpenAiError::ToolsNotList)));
    assert_eq!(nulled, object(json!({"tools": null})));
}

fn dict_response_schema_refs() {
    let refs = |format: Value, template: &str| response_format_refs(Some(&object(format)), Some(template));
    assert_eq!(response_format_refs(None, Some("{model}")).unwrap(), None);
    let plain = object(json!({"type": "json_schema", "json_schema": 3}));
    assert_eq!(response_format_refs(Some(&plain), None).unwrap(), Some(plain.clone()));
    let other = json!({"type": "json_object", "json_schema": 3});
    assert_eq!(refs(other.clone(), "{x}").unwrap(), Some(object(other)));
    let format = json!({"type": "json_schema", "extra": 1, "json_schema": {"name": "n", "schema": {
        "$ref": "#/$defs/Root", "title": "t",
        "$defs": {"Root": {"anyOf": [{"$ref": "Leaf"}, [{"$ref": "a/b/Item"}]], "default": {"$ref": "x/"}}}
    }}});
    let expected = json!({"type": "json_schema", "extra": 1, "json_schema": {"name": "n", "schema": {
        "$ref": "{m}/Root!", "title": "t",
        "$defs": {"Root": {"anyOf": [{"$ref": "{m}/Leaf!"}, [{"$ref": "{m}/Item!"}]], "default": {"$ref": "{m}/!"}}}
    }}});
    let input = object(format.clone());
    let rewritten = response_format_refs(Some(&input), Some("{{m}}/{model}!")).unwrap();
    assert_eq!((rewritten, input), (Some(object(expected)), object(format)));
    let schema = |inner: Value| json!({"type": "json_schema", "json_schema": {"schema": inner}});
    assert_eq!(refs(schema(json!({"a": 1})), "{bad").unwrap(), Some(object(schema(json!({"a": 1})))));
    let failure = |format: Value, template: &str| refs(format, template).unwrap_err();
    let missing = failure(json!({"type": "json_schema"}), "{model}");
    assert!(matches!(missing, OpenAiError::MissingKey { key: "json_schema" }));
    let text = failure(json!({"type": "json_schema", "json_schema": "s"}), "{model}");
    assert!(matches!(text, OpenAiError::NotSubscriptable { key: "json_schema" }));
    let no_schema = failure(json!({"type": "json_schema", "json_schema": {}}), "{model}");
    assert!(matches!(no_schema, OpenAiError::MissingKey { key: "schema" }));
    let number = failure(schema(json!([{"$ref": 1}])), "{model}");
    assert!(matches!(number, OpenAiError::RefNotText));
    let named = failure(schema(json!({"$ref": "a"})), "{name}");
    assert!(matches!(named, OpenAiError::TemplateField { field } if field == "name"));
    let positional = failure(schema(json!({"$ref": "a"})), "{0}");
    assert!(matches!(positional, OpenAiError::TemplatePosition));
    assert!(matches!(failure(schema(json!({"$ref": "a"})), "x}"), OpenAiError::TemplateBrace));
    assert!(matches!(failure(schema(json!({"$ref": "a"})), "{model"), OpenAiError::TemplateBrace));
}

fn hidden_metadata_projection() {
    let carrier = |kind: ResponseKind, hidden: Option<Value>| HiddenCarrier {
        kind,
        hidden_params: hidden.map(object),
    };
    let mut stamped = carrier(ResponseKind::Dict, Some(json!({"model_id": "m"})));
    assert_eq!(hidden_params(&mut stamped, true), object(json!({"model_id": "m"})));
    assert_eq!(stamped, carrier(ResponseKind::Dict, Some(json!({"model_id": "m"}))));
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
    let calls = history(json!([{"role": "assistant", "tool_calls": null}, {"role": "user", "tool_calls": []}]));
    assert!(has_tool_calls(&calls));
    assert!(!has_tool_calls(&history(json!([{"role": "assistant", "tool_calls": null}]))));
    let thinking = |messages: Value| assistant_has_thinking(&history(messages));
    assert!(thinking(json!([{"role": "assistant", "thinking_blocks": [{"type": "thinking"}]}])));
    assert!(thinking(json!([{"role": "assistant", "thinking_blocks": 5}])));
    assert!(!thinking(json!([{"role": "assistant", "thinking_blocks": []}, {"role": "assistant", "thinking_blocks": ""}])));
    assert!(!thinking(json!([{"role": "user", "thinking_blocks": [1]}, {"role": "assistant"}])));
    let unthought = |messages: Value| last_tool_call_unthought(&history(messages));
    assert!(!unthought(json!([{"role": "user", "tool_calls": []}, {"role": "assistant"}])));
    assert!(unthought(json!([{"role": "assistant", "tool_calls": [], "thinking_blocks": null}])));
    assert!(unthought(json!([
        {"role": "assistant", "tool_calls": [], "thinking_blocks": [1]},
        {"role": "assistant", "tool_calls": [], "thinking_blocks": {}}
    ])));
    assert!(!unthought(json!([{"role": "assistant", "tool_calls": [], "thinking_blocks": [1]}])));
    assert!(!unthought(json!([{"role": "assistant", "tool_calls": [], "thinking_blocks": 7}])));
}

fn dummy_tool_literal() {
    let expected = json!([{"type": "function", "function": {
        "name": "dummy_tool",
        "description": "This is a dummy tool call",
        "parameters": {"type": "object", "properties": {}}
    }}]);
    assert_eq!(serde_json::to_value(DUMMY_TOOLS).unwrap(), expected);
}

fn modify_params_default_and_env() {
    let modify = |source: &str, environment: &[(&str, &str)]| load_settings(source, environment).0;
    assert!(!modify("", &[]));
    assert!(!modify("", &[("LITELLM_MODIFY_PARAMS", "")]));
    assert!(modify("", &[("LITELLM_MODIFY_PARAMS", "false")]));
    assert!(modify("", &[("LITELLM_MODIFY_PARAMS", "0")]));
    assert!(modify("", &[("LITELLM_MODIFY_PARAMS", " ")]));
    let configured_off = "litellm_settings:\n  modify_params: false\n";
    assert!(!modify(configured_off, &[("LITELLM_MODIFY_PARAMS", "true")]));
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
#[case::append_tools(append_tools_cases)]
#[case::dict_response_schema_refs(dict_response_schema_refs)]
#[case::hidden_metadata_projection(hidden_metadata_projection)]
#[case::history_tool_and_thinking_predicates(history_tool_and_thinking_predicates)]
#[case::dummy_tool_literal(dummy_tool_literal)]
#[case::modify_params_default_and_env(modify_params_default_and_env)]
#[case::base_responses_initializer(base_responses_initializer)]
#[case::base_image_edit_initializer(base_image_edit_initializer)]
fn base_helpers_cases(#[case] check: fn()) {
    check();
}
