#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{
    MockChatChoice, MockChatMessage, MockChatObject, MockResponse, MockToolCall, MockUsage,
    Settings,
};

fn load(source: &str, environment: &BTreeMap<String, String>) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .map(|config| config.sections.mock_response)
}

fn deployment(source: &str) -> Result<Option<MockResponse>, ConfigError> {
    let settings = load(source, &BTreeMap::new())?;
    Ok(settings.deployments.first().cloned().unwrap_or_default())
}

#[test]
fn reads_a_string_mock_response() {
    let value = deployment(
        "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response: This is a mock request\n",
    )
    .expect("config");
    assert_eq!(
        value,
        Some(MockResponse::Text(String::from("This is a mock request")))
    );
}

#[test]
fn reads_an_embedding_vector() {
    let value = deployment(
        "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response: [0.1, 0.2]\n",
    )
    .expect("config");
    assert_eq!(
        value,
        Some(MockResponse::Numbers(vec![
            serde_json::Number::from_f64(0.1).expect("number"),
            serde_json::Number::from_f64(0.2).expect("number"),
        ]))
    );
}

#[test]
fn reads_a_response_object_with_litellm_defaults() {
    let value = deployment(
        "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response:\n        id: given\n        created: 5\n        choices:\n          - message: {content: hello}\n",
    )
    .expect("config");
    let Some(MockResponse::Object(object)) = value else {
        panic!("expected an object");
    };
    assert_eq!(object.id.as_deref(), Some("given"));
    assert_eq!(object.created, Some(5));
    assert_eq!(
        object.choices,
        Some(vec![MockChatChoice {
            index: 0,
            message: MockChatMessage {
                role: String::from("assistant"),
                content: Some(String::from("hello")),
                tool_calls: None,
            },
            finish_reason: Some(String::from("stop")),
        }])
    );
}

#[test]
fn reads_tool_calls_and_usage_in_a_response_object() {
    let value = deployment(
        "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response:\n        choices:\n          - index: 1\n            message:\n              tool_calls:\n                - {id: t, type: function, function: {name: f, arguments: '{}'}}\n        usage: {prompt_tokens: 1, completion_tokens: 2, total_tokens: 3}\n",
    )
    .expect("config");
    let Some(MockResponse::Object(MockChatObject {
        choices: Some(choices),
        usage: Some(usage),
        ..
    })) = value
    else {
        panic!("expected an object with choices and usage");
    };
    assert_eq!(
        choices
            .first()
            .and_then(|choice| choice.message.tool_calls.clone()),
        Some(vec![MockToolCall {
            id: String::from("t"),
            kind: String::from("function"),
            function: super::MockFunctionCall {
                name: String::from("f"),
                arguments: String::from("{}"),
            },
        }])
    );
    assert_eq!(
        usage,
        MockUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
        }
    );
}

#[test]
fn resolves_an_environment_reference() {
    let environment = BTreeMap::from([(String::from("MOCK_BODY"), String::from("from-env"))]);
    let settings = load(
        "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response: os.environ/MOCK_BODY\n",
        &environment,
    )
    .expect("config");
    assert_eq!(
        settings.deployments.into_iter().next().flatten(),
        Some(MockResponse::Text(String::from("from-env")))
    );
}

#[rstest]
#[case::missing(
    "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response: os.environ/UNSET_MOCK\n",
    "missing_environment"
)]
#[case::not_a_mock_value(
    "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response: true\n",
    "yaml"
)]
#[case::unknown_object_key(
    "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response: {unexpected: field}\n",
    "yaml"
)]
fn rejects_invalid_mock_values(#[case] source: &str, #[case] category: &str) {
    let error = load(source, &BTreeMap::new()).expect_err("invalid mock value");
    assert_eq!(error.category(), category);
}

// LiteLLM's router would merge these deployment parameters into the call; this section loads only mock_response
#[rstest]
#[case::mock_timeout("mock_timeout: true")]
#[case::mock_delay("mock_delay: 0.5")]
fn an_operator_mock_wait_parameter_is_not_loaded(#[case] parameter: &str) {
    let source = format!(
        "model_list:\n  - model_name: mock\n    litellm_params:\n      mock_response: hi\n      {parameter}\n"
    );
    let error = load(&source, &BTreeMap::new()).expect_err("unsupported parameter");
    let ConfigError::Unsupported { paths } = error else {
        panic!("expected an unsupported key");
    };
    let name = parameter.split(':').next().expect("parameter name");
    assert!(paths.iter().any(|path| path.ends_with(name)), "{paths:?}");
}

#[test]
fn keeps_the_position_of_each_deployment() {
    let settings = load(
        "model_list:\n  - model_name: plain\n    litellm_params: {}\n  - model_name: mock\n    litellm_params:\n      mock_response: hi\n",
        &BTreeMap::new(),
    )
    .expect("config");
    assert_eq!(settings.deployments.len(), 2);
    assert_eq!(settings.deployments.first().cloned().flatten(), None);
    assert_eq!(
        settings.deployments.get(1).cloned().flatten(),
        Some(MockResponse::Text(String::from("hi")))
    );
}

#[rstest]
#[case::defaults(BTreeMap::new(), 10, 20)]
#[case::environment(
    BTreeMap::from([
        (String::from("DEFAULT_MOCK_RESPONSE_PROMPT_TOKEN_COUNT"), String::from("3")),
        (String::from("DEFAULT_MOCK_RESPONSE_COMPLETION_TOKEN_COUNT"), String::from("4")),
    ]),
    3,
    4
)]
fn reads_usage_token_count_defaults(
    #[case] environment: BTreeMap<String, String>,
    #[case] prompt: u64,
    #[case] completion: u64,
) {
    let settings = load("model_list: []\n", &environment).expect("config");
    assert_eq!(settings.usage.prompt_tokens, prompt);
    assert_eq!(settings.usage.completion_tokens, completion);
}

#[test]
fn rejects_an_invalid_usage_token_count() {
    let environment = BTreeMap::from([(
        String::from("DEFAULT_MOCK_RESPONSE_PROMPT_TOKEN_COUNT"),
        String::from("many"),
    )]);
    assert!(matches!(
        load("model_list: []\n", &environment),
        Err(ConfigError::Environment { .. })
    ));
}
