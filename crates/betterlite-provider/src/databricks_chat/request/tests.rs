#![cfg(test)]

use std::future::Future;

use async_trait::async_trait;
use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_core::api::messages_native_transform::capabilities::{
    ADAPTIVE_THINKING, LegacyShapes, Thinking, ThinkingCapabilities,
};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Number, Value, json};

use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::oauth::{OauthReply, OauthTokenRequest, OauthTokenSource};
use crate::databricks_chat::request::params::map_openai_params;
use crate::databricks_chat::request::tools::convert_anthropic_tool;
use crate::databricks_chat::request::{
    EnvironmentRequest, SUPPORTED_OPENAI_PARAMS, get_complete_url, get_config, get_required_params,
    pop_user_agent, pop_user_agent_parameter, should_fake_stream, validate_environment,
};

fn object(value: Value) -> Object {
    serde_json::from_value(value).expect("the fixture is an object")
}

// get_complete_url: two dots in the model after the provider prefix turn the AI gateway on
#[rstest]
#[case(
    Some("https://adb-1.cloud.databricks.com/serving-endpoints"),
    "databricks-meta-llama-3-1-70b",
    "https://adb-1.cloud.databricks.com/serving-endpoints/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/serving-endpoints"),
    "databricks/anthropic.claude-3-5-sonnet-2",
    "https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/"),
    "databricks/anthropic.claude-3-5-sonnet-2",
    "https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/"),
    "databricks/anthropic.claude-3-5-sonnet-2",
    "https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/serving-endpoints/"),
    "databricks/one.dot",
    "https://adb-1.cloud.databricks.com/serving-endpoints//chat/completions"
)]
fn the_complete_url_follows_the_gateway_rule(
    #[case] api_base: Option<&str>,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        get_complete_url(api_base, model).expect("the url builds"),
        expected
    );
}

#[test]
fn a_missing_api_base_is_a_bad_request() {
    let error = get_complete_url(None, "databricks-meta-llama-3-1-70b")
        .expect_err("the url needs an api base");
    assert_eq!(error.status_code(), 400);
}

#[test]
fn the_user_agent_wins_and_keeps_the_databricks_one() {
    let mut params = object(json!({"user_agent": "first", "databricks_user_agent": "second"}));
    assert_eq!(
        pop_user_agent_parameter(&mut params).expect("the user agents pop"),
        Some(String::from("first"))
    );
    assert_eq!(
        params.get("databricks_user_agent"),
        Some(&Json::Text(String::from("second")))
    );
}

#[test]
fn the_databricks_user_agent_is_popped_when_the_user_agent_is_absent() {
    let mut params = object(json!({"databricks_user_agent": "second"}));
    assert_eq!(
        pop_user_agent_parameter(&mut params).expect("the user agent pops"),
        Some(String::from("second"))
    );
    assert!(params.is_empty());
}

#[rstest]
#[case(json!(null))]
#[case(json!(""))]
fn an_empty_user_agent_falls_through_to_the_databricks_one(#[case] value: Value) {
    let mut params = object(json!({"user_agent": value, "databricks_user_agent": "second"}));
    assert_eq!(
        pop_user_agent_parameter(&mut params).expect("the user agents pop"),
        Some(String::from("second"))
    );
    assert!(params.is_empty());
}

#[rstest]
#[case(json!({"user_agent": 5}), "user_agent")]
#[case(json!({"user_agent": []}), "user_agent")]
#[case(json!({"databricks_user_agent": {}}), "databricks_user_agent")]
fn a_non_string_user_agent_is_a_bad_request(#[case] fixture: Value, #[case] name: &str) {
    let mut params = object(fixture);
    let error = pop_user_agent_parameter(&mut params).expect_err("the user agent is invalid");
    assert_eq!(error.status_code(), 400);
    assert!(
        error
            .to_string()
            .contains(&format!("the {name} optional parameter must be a string")),
        "{error}"
    );
}

#[test]
fn a_popped_user_agent_leaves_no_parameter_behind() {
    let mut params = object(json!({"user_agent": "first", "temperature": 1}));
    pop_user_agent_parameter(&mut params).expect("the user agent pops");
    assert_eq!(params.len(), 1);
    assert!(params.contains_key("temperature"));
}

#[test]
fn popping_a_user_agent_that_is_not_there_is_none() {
    let mut params = Object::new();
    assert_eq!(
        pop_user_agent(&mut params, "user_agent").expect("an absent user agent is fine"),
        None
    );
}

const VERSION: &str = "1.105.1";

// an api key request never asks for an OAuth token, so the source refuses instead of sending
struct NoTokens;

#[async_trait]
impl OauthTokenSource for NoTokens {
    async fn send(&self, _request: &OauthTokenRequest) -> OauthReply {
        OauthReply::Transport(String::from("the request carries an api key"))
    }
}

async fn user_agent_of(
    litellm_user_agent: Option<&str>,
    env_user_agent: Option<&str>,
    user_agent_override: Option<&str>,
) -> String {
    let key = SecretString::from(String::from("dapi-key"));
    let env = DatabricksEnv {
        client_id: None,
        client_secret: None,
        api_base: None,
        user_agent_override: user_agent_override.map(str::to_owned),
        version: VERSION.to_owned(),
    };
    let request = EnvironmentRequest {
        api_key: Some(&key),
        api_base: Some("https://adb-1.cloud.databricks.com"),
        headers: None,
        litellm_user_agent,
        env_user_agent,
    };
    let mut optional = Object::new();
    let headers = validate_environment(request, &mut optional, &env, &NoTokens)
        .await
        .expect("the headers build");
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    headers
        .get("User-Agent")
        .cloned()
        .expect("the headers carry a user agent")
}

#[tokio::test]
async fn the_user_agent_falls_through_litellm_then_the_environment() {
    assert_eq!(
        user_agent_of(Some("lit"), None, None).await,
        "lit_litellm/1.105.1"
    );
    assert_eq!(
        user_agent_of(Some(""), Some("env"), None).await,
        "env_litellm/1.105.1"
    );
    assert_eq!(
        user_agent_of(None, None, Some("ovr")).await,
        "ovr_litellm/1.105.1"
    );
    assert_eq!(user_agent_of(Some(""), None, None).await, "litellm/1.105.1");
}

#[rstest]
#[case(json!(null), false)]
#[case(json!({"type": "json_object"}), true)]
#[case(json!("json"), true)]
fn should_fake_stream_follows_a_set_response_format(#[case] value: Value, #[case] expected: bool) {
    let params = object(json!({"response_format": value}));
    assert_eq!(should_fake_stream(&params), expected);
}

#[test]
fn the_config_carries_the_default_chat_max_tokens() {
    assert_eq!(
        get_config(4096).get("max_tokens"),
        Some(&Json::Number(Number::from(4096_i64)))
    );
}

#[test]
fn the_required_params_name_the_api_key_and_base() {
    let params = get_required_params();
    let names: Vec<&str> = params.iter().map(|field| field.field_name).collect();
    assert_eq!(names, ["api_key", "api_base"]);
}

#[test]
fn the_supported_params_are_the_openai_subset_databricks_maps() {
    assert!(SUPPORTED_OPENAI_PARAMS.contains(&"response_format"));
    assert!(SUPPORTED_OPENAI_PARAMS.contains(&"thinking"));
    assert!(!SUPPORTED_OPENAI_PARAMS.contains(&"presence_penalty"));
}

// The capability answers map_openai_params asks for: the static effort budgets of a
// non-adaptive model, and an adaptive model whose legacy thinking becomes an output_config.
struct FakeCapabilities {
    adaptive: bool,
}

impl ThinkingCapabilities for FakeCapabilities {
    fn supports(
        &mut self,
        model: &str,
        key: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        let matched = self.adaptive && model.contains("adaptive") && key == ADAPTIVE_THINKING;
        async move { matched }
    }

    fn map_effort<'seam>(
        &'seam mut self,
        _model: &'seam str,
        effort: &'seam str,
        _provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam {
        async move { effort_budget(effort) }
    }

    fn validate_effort(
        &mut self,
        _model: &str,
        _effort: Option<&str>,
        _provider: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        async { None }
    }

    fn supports_effort_param(
        &mut self,
        _model: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        async { false }
    }

    fn drop_disabled_thinking(
        &mut self,
        _model: &str,
        _params: &mut OptionalParams,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        async { false }
    }

    // B569's adaptive translation: a legacy thinking block becomes an output_config effort
    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam {
        let adaptive = self.adaptive && request.model.contains("adaptive");
        async move {
            if !adaptive || request.params.thinking.is_none() {
                return Ok(());
            }
            request.params.thinking = None;
            request.params.output_config = Some(translated_output_config());
            Ok(())
        }
    }
}

// _map_reasoning_effort's static budgets for a model that is not adaptive
fn effort_budget(effort: &str) -> Result<Option<Thinking>, ReasoningEffortError> {
    let budget: i64 = match effort {
        "none" => return Ok(None),
        "minimal" | "low" => 1024,
        "medium" => 2048,
        "high" => 4096,
        "xhigh" => 8192,
        "max" => 16384,
        _ => {
            return Err(ReasoningEffortError::Unmapped {
                value: effort.to_owned(),
                model: String::new(),
                provider: String::new(),
            });
        }
    };
    let mut thinking = Thinking::new();
    thinking.insert(
        "type".to_owned(),
        DecisionJson::Text(String::from("enabled")),
    );
    thinking.insert(
        "budget_tokens".to_owned(),
        DecisionJson::Number(Number::from(budget)),
    );
    Ok(Some(thinking))
}

fn translated_output_config() -> DecisionJson {
    let mut fields = IndexMap::new();
    fields.insert("effort".to_owned(), DecisionJson::Text(String::from("low")));
    DecisionJson::Object(fields)
}

async fn mapped(model: &str, non_default: Value, adaptive: bool) -> Object {
    let mut optional = Object::new();
    let mut capabilities = FakeCapabilities { adaptive };
    map_openai_params(
        &object(non_default),
        &mut optional,
        model,
        true,
        &mut capabilities,
    )
    .await
    .expect("the params map");
    optional
}

fn assert_params(params: &Object, expected: Value) {
    assert_eq!(
        serde_json::to_value(params).expect("the params render"),
        expected
    );
}

#[tokio::test]
async fn a_claude_tool_gains_an_empty_properties_map_and_its_description() {
    let params = mapped(
        "databricks-claude-3-5-sonnet",
        json!({
            "tools": [{"type": "function", "function": {"name": "f", "description": "d", "parameters": {"type": "object"}}}],
            "tool_choice": "auto",
            "temperature": 0.2
        }),
        false,
    )
    .await;
    assert_params(
        &params,
        json!({
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {"type": "object", "properties": {}}, "description": "d"}}],
            "tool_choice": "auto",
            "temperature": 0.2
        }),
    );
}

#[tokio::test]
async fn a_json_schema_response_format_becomes_the_json_tool() {
    let params = mapped(
        "databricks-claude-3-5-sonnet",
        json!({
            "response_format": {"type": "json_schema", "json_schema": {"name": "out", "schema": {"type": "object", "properties": {"a": {"type": "string"}}}}}
        }),
        false,
    )
    .await;
    assert_params(
        &params,
        json!({
            "tools": [{"type": "function", "function": {"name": "json_tool_call", "parameters": {"type": "object", "properties": {"a": {"type": "string"}}}}}],
            "json_mode": true,
            "tool_choice": {"type": "function", "function": {"name": "json_tool_call"}}
        }),
    );
}

#[tokio::test]
async fn a_json_object_response_format_is_only_removed() {
    let params = mapped(
        "databricks-claude-3-5-sonnet",
        json!({"response_format": {"type": "json_object"}, "temperature": 0.3}),
        false,
    )
    .await;
    assert_params(&params, json!({"temperature": 0.3}));
}

#[tokio::test]
async fn a_json_schema_format_with_thinking_installs_no_tool_choice() {
    let params = mapped(
        "databricks-claude-3-5-sonnet",
        json!({
            "response_format": {"type": "json_schema", "json_schema": {"name": "out", "schema": {"type": "object", "properties": {"a": {"type": "string"}}}}},
            "thinking": {"type": "enabled", "budget_tokens": 2048}
        }),
        false,
    )
    .await;
    assert_params(
        &params,
        json!({
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "tools": [{"type": "function", "function": {"name": "json_tool_call", "parameters": {"type": "object", "properties": {"a": {"type": "string"}}}}}],
            "json_mode": true,
            "max_tokens": 6144
        }),
    );
}

#[tokio::test]
async fn a_non_claude_model_keeps_its_tools_and_response_format() {
    let params = mapped(
        "databricks-meta-llama-3-1-70b",
        json!({
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}],
            "response_format": {"type": "json_object"},
            "max_completion_tokens": 50,
            "temperature": 0.2
        }),
        false,
    )
    .await;
    assert_params(
        &params,
        json!({
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}],
            "response_format": {"type": "json_object"},
            "temperature": 0.2,
            "max_tokens": 50
        }),
    );
}

#[tokio::test]
async fn an_uppercase_claude_model_keeps_response_format_but_maps_the_effort() {
    let params = mapped(
        "databricks-Claude-3",
        json!({"response_format": {"type": "json_object"}, "reasoning_effort": "low"}),
        false,
    )
    .await;
    assert_params(
        &params,
        json!({
            "response_format": {"type": "json_object"},
            "thinking": {"type": "enabled", "budget_tokens": 1024},
            "max_tokens": 5120
        }),
    );
}

#[tokio::test]
async fn a_reasoning_effort_fills_the_thinking_budget() {
    for (effort, budget, max_tokens) in [
        ("minimal", 1024, 5120),
        ("low", 1024, 5120),
        ("medium", 2048, 6144),
        ("high", 4096, 8192),
        ("xhigh", 8192, 12288),
        ("max", 16384, 20480),
    ] {
        let params = mapped(
            "databricks-claude-3-5-sonnet",
            json!({"reasoning_effort": effort}),
            false,
        )
        .await;
        assert_params(
            &params,
            json!({"thinking": {"type": "enabled", "budget_tokens": budget}, "max_tokens": max_tokens}),
        );
    }
}

#[tokio::test]
async fn an_adaptive_model_also_gets_an_output_config_effort() {
    for (effort, level, budget, max_tokens) in [
        ("minimal", "low", 1024, 5120),
        ("low", "low", 1024, 5120),
        ("medium", "medium", 2048, 6144),
        ("high", "high", 4096, 8192),
        ("xhigh", "xhigh", 8192, 12288),
        ("max", "max", 16384, 20480),
    ] {
        let params = mapped(
            "databricks-claude-adaptive",
            json!({"reasoning_effort": effort}),
            true,
        )
        .await;
        assert_params(
            &params,
            json!({
                "thinking": {"type": "enabled", "budget_tokens": budget},
                "output_config": {"effort": level},
                "max_tokens": max_tokens
            }),
        );
    }
}

#[tokio::test]
async fn an_adaptive_model_translates_a_legacy_thinking_block() {
    let params = mapped(
        "databricks-claude-adaptive",
        json!({"thinking": {"type": "enabled", "budget_tokens": 2048}}),
        true,
    )
    .await;
    assert_params(&params, json!({"output_config": {"effort": "low"}}));
}

#[tokio::test]
async fn a_none_effort_drops_thinking() {
    let params = mapped(
        "databricks-claude-3-5-sonnet",
        json!({"reasoning_effort": "none", "thinking": {"type": "enabled", "budget_tokens": 2048}}),
        false,
    )
    .await;
    assert_params(&params, json!({}));
}

#[tokio::test]
async fn an_unknown_reasoning_effort_is_a_bad_request() {
    let mut optional = Object::new();
    let error = map_openai_params(
        &object(json!({"reasoning_effort": "banana"})),
        &mut optional,
        "databricks-claude-3-5-sonnet",
        true,
        &mut FakeCapabilities { adaptive: false },
    )
    .await
    .expect_err("banana is not an effort");
    assert_eq!(error.status_code(), 400);
}

#[tokio::test]
async fn a_non_string_reasoning_effort_is_a_bad_request() {
    let mut optional = Object::new();
    let error = map_openai_params(
        &object(json!({"reasoning_effort": 5})),
        &mut optional,
        "databricks-claude-3-5-sonnet",
        true,
        &mut FakeCapabilities { adaptive: false },
    )
    .await
    .expect_err("a number is not an effort");
    assert_eq!(error.status_code(), 400);
}

#[tokio::test]
async fn max_completion_tokens_wins_over_max_tokens() {
    let params = mapped(
        "databricks-claude-3-5-sonnet",
        json!({"max_tokens": 10, "max_completion_tokens": 20}),
        false,
    )
    .await;
    assert_params(&params, json!({"max_tokens": 20}));
}

#[tokio::test]
async fn a_thinking_budget_raises_the_default_max_tokens() {
    let params = mapped(
        "databricks-claude-3-5-sonnet",
        json!({"thinking": {"type": "enabled", "budget_tokens": 2048}}),
        false,
    )
    .await;
    assert_params(
        &params,
        json!({"thinking": {"type": "enabled", "budget_tokens": 2048}, "max_tokens": 6144}),
    );
}

#[tokio::test]
async fn a_non_claude_model_keeps_its_reasoning_effort() {
    let params = mapped(
        "databricks-meta-llama-3-1-70b",
        json!({"reasoning_effort": "low"}),
        false,
    )
    .await;
    assert_params(&params, json!({"reasoning_effort": "low"}));
}

// Python's `tool.get("input_schema") or {}` turns any falsy schema into an empty parameters map
#[test]
fn a_tool_without_a_truthy_schema_gets_an_empty_parameters_map() {
    let tool = object(json!({"name": "f", "input_schema": null}));
    let converted = convert_anthropic_tool(&tool).expect("the tool converts");
    assert_eq!(
        serde_json::to_value(&converted).expect("the tool renders"),
        json!({"type": "function", "function": {"name": "f", "parameters": {}}})
    );
}

// Only a description that exists is added, so an explicit null leaves the key out
#[test]
fn a_null_description_is_left_out() {
    let tool = object(json!({
        "name": "f",
        "input_schema": {"type": "object", "properties": {}},
        "description": null
    }));
    let converted = convert_anthropic_tool(&tool).expect("the tool converts");
    assert_eq!(
        serde_json::to_value(&converted).expect("the tool renders"),
        json!({
            "type": "function",
            "function": {
                "name": "f",
                "parameters": {"type": "object", "properties": {}}
            }
        })
    );
}
