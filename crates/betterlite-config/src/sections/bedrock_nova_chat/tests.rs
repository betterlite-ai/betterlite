#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json as JsonData;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{DEFAULT_API_BASE, NovaChatValues, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<Settings, ConfigError> {
    load_with(source, &[])
}

fn load_with(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .bedrock_nova_chat)
}

fn deployment(fields: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: amazon_nova/unit, amazon_nova: {{{fields}}}}}\n"
    )
}

fn global(fields: &str) -> String {
    format!("litellm_settings: {{amazon_nova: {{{fields}}}}}\n")
}

fn json(source: &str) -> JsonData {
    serde_json::from_str(source).expect("the fixture JSON parses")
}

fn assert_integer_field(key: &str, read: fn(&NovaChatValues) -> Option<i64>) {
    assert_precedence(key, read);
    let zero = load(&deployment(&format!("{key}: 0"))).unwrap();
    assert_eq!(
        read(zero.deployments.first().unwrap()),
        Some(0),
        "{key} keeps an explicit zero"
    );
    let variable = format!("NOVA_CONFIG_{}", key.to_uppercase());
    let referenced = load_with(
        &deployment(&format!("{key}: os.environ/{variable}")),
        &[(variable.as_str(), "16")],
    )
    .unwrap();
    assert_eq!(
        read(referenced.deployments.first().unwrap()),
        Some(16),
        "{key} reads the environment"
    );
    let expected = format!("model_list.0.litellm_params.amazon_nova.{key}");
    let missing = load(&deployment(&format!("{key}: os.environ/ABSENT_{variable}"))).unwrap_err();
    let absent = format!("ABSENT_{variable}");
    assert!(
        matches!(&missing, ConfigError::MissingEnvironment { variable: name, path } if *name == absent && *path == expected),
        "{missing}"
    );
    let invalid = load(&deployment(&format!("{key}: 'many'"))).unwrap_err();
    assert!(
        matches!(&invalid, ConfigError::InvalidInteger { path, .. } if *path == expected),
        "{invalid}"
    );
}

fn assert_precedence(key: &str, read: fn(&NovaChatValues) -> Option<i64>) {
    let default = load("").unwrap();
    assert_eq!(read(&default.litellm_settings), None, "{key} default");
    assert!(default.deployments.is_empty());
    let only_deployment = load(&deployment(&format!("{key}: 64"))).unwrap();
    assert_eq!(
        read(only_deployment.deployments.first().unwrap()),
        Some(64),
        "{key} deployment"
    );
    let only_global = load(&global(&format!("{key}: 128"))).unwrap();
    assert_eq!(
        read(&only_global.litellm_settings),
        Some(128),
        "{key} global"
    );
    let configured = load(&format!(
        "{}{}",
        deployment(&format!("{key}: 64")),
        global(&format!("{key}: 128"))
    ))
    .unwrap();
    assert_eq!(
        read(configured.deployments.first().unwrap()),
        Some(64),
        "{key} keeps the deployment value"
    );
    assert_eq!(
        read(&configured.litellm_settings),
        Some(128),
        "{key} keeps the module value"
    );
    assert_eq!(
        read(&configured.resolve(0)),
        Some(64),
        "{key} resolves the deployment over the module"
    );
    assert_eq!(
        read(&configured.resolve(1)),
        Some(128),
        "{key} without a deployment"
    );
}

fn loads_amazon_nova_api_base() {
    let default = load("").unwrap();
    assert_eq!(default.api_base_environment, None);
    assert_eq!(default.api_base(), DEFAULT_API_BASE);
    let configured =
        load_with("", &[("AMAZON_NOVA_API_BASE", "https://proxy.example/v1/")]).unwrap();
    assert_eq!(
        configured.api_base_environment.as_deref(),
        Some("https://proxy.example/v1/")
    );
    assert_eq!(configured.api_base(), "https://proxy.example/v1/");
    let blank = load_with("", &[("AMAZON_NOVA_API_BASE", "")]).unwrap();
    assert_eq!(
        blank.api_base_environment, None,
        "an empty variable is unset"
    );
    assert_eq!(blank.api_base(), DEFAULT_API_BASE);
}

fn loads_amazon_nova_api_key() {
    let default = load("").unwrap();
    assert!(default.api_key_environment.is_none());
    assert!(default.api_key().is_none());
    let configured = load_with("", &[("AMAZON_NOVA_API_KEY", "env-key")]).unwrap();
    let environment_key = configured
        .api_key_environment
        .as_ref()
        .expect("the environment key");
    assert_eq!(environment_key.expose_secret(), "env-key");
    let resolved = configured.api_key().expect("the environment key resolves");
    assert_eq!(resolved.expose_secret(), "env-key");
    let blank = load_with("", &[("AMAZON_NOVA_API_KEY", "")]).unwrap();
    assert!(
        blank.api_key_environment.is_none(),
        "an empty variable is unset"
    );
    assert!(blank.api_key().is_none());
}

fn loads_global_amazon_nova_api_key() {
    let configured = load("litellm_settings: {amazon_nova_api_key: global-key}\n").unwrap();
    let global_key = configured
        .amazon_nova_api_key
        .as_ref()
        .expect("the global key");
    assert_eq!(global_key.expose_secret(), "global-key");
    assert_eq!(
        configured
            .api_key()
            .expect("the global key resolves")
            .expose_secret(),
        "global-key"
    );
    assert!(!format!("{configured:?}").contains("global-key"));
    let both = load_with(
        "litellm_settings: {amazon_nova_api_key: global-key}\n",
        &[("AMAZON_NOVA_API_KEY", "env-key")],
    )
    .unwrap();
    assert_eq!(
        both.api_key().expect("the global key wins").expose_secret(),
        "global-key"
    );
    let referenced = load_with(
        "litellm_settings: {amazon_nova_api_key: os.environ/NOVA_GLOBAL_KEY}\n",
        &[("NOVA_GLOBAL_KEY", "resolved-key")],
    )
    .unwrap();
    let resolved = referenced
        .amazon_nova_api_key
        .as_ref()
        .expect("the reference resolves");
    assert_eq!(resolved.expose_secret(), "resolved-key");
}

fn loads_amazonnovachatconfig_max_completion_tokens() {
    assert_integer_field("max_completion_tokens", |values| {
        values.max_completion_tokens
    });
}

fn loads_amazonnovachatconfig_max_tokens() {
    assert_integer_field("max_tokens", |values| values.max_tokens);
}

fn loads_amazonnovachatconfig_temperature() {
    assert_integer_field("temperature", |values| values.temperature);
}

fn loads_amazonnovachatconfig_top_p() {
    assert_integer_field("top_p", |values| values.top_p);
}

fn loads_amazonnovachatconfig_metadata() {
    assert_integer_field("metadata", |values| values.metadata);
}

fn loads_amazonnovachatconfig_tools() {
    let default = load("").unwrap();
    assert_eq!(default.litellm_settings.tools, None);
    let only_deployment = load(&deployment(
        "tools: [{type: function, function: {name: lookup}}]",
    ))
    .unwrap();
    assert_eq!(
        only_deployment.deployments.first().unwrap().tools,
        Some(vec![json(
            r#"{"type": "function", "function": {"name": "lookup"}}"#
        )])
    );
    let configured = load(&format!(
        "{}{}",
        deployment("tools: []"),
        global("tools: [low]")
    ))
    .unwrap();
    assert_eq!(
        configured.resolve(0).tools,
        Some(Vec::new()),
        "an explicit empty list is used as given"
    );
    assert_eq!(
        configured.resolve(1).tools,
        Some(vec![json(r#""low""#)]),
        "without a deployment"
    );
}

fn loads_amazonnovachatconfig_reasoning_effort() {
    let default = load("").unwrap();
    assert_eq!(default.litellm_settings.reasoning_effort, None);
    let only_global = load(&global("reasoning_effort: [low, high]")).unwrap();
    assert_eq!(
        only_global.litellm_settings.reasoning_effort,
        Some(vec![json(r#""low""#), json(r#""high""#)])
    );
    let configured = load(&format!(
        "{}{}",
        deployment("reasoning_effort: [minimal]"),
        global("reasoning_effort: [high]")
    ))
    .unwrap();
    assert_eq!(
        configured.resolve(0).reasoning_effort,
        Some(vec![json(r#""minimal""#)]),
        "the deployment value wins"
    );
    assert_eq!(
        configured.resolve(1).reasoning_effort,
        Some(vec![json(r#""high""#)]),
        "without a deployment"
    );
}

fn rejects_unrelated_key() {
    let nested = load(&global("extra_knob: 1")).unwrap_err();
    assert!(
        matches!(&nested, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.amazon_nova.extra_knob"]),
        "{nested}"
    );
    let in_deployment = load(&deployment("extra_knob: 1")).unwrap_err();
    assert!(
        matches!(&in_deployment, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.amazon_nova.extra_knob"]),
        "{in_deployment}"
    );
    let unrelated = load("litellm_settings: {max_tokens_extra: 1}\n").unwrap_err();
    assert!(
        matches!(&unrelated, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.max_tokens_extra"]),
        "{unrelated}"
    );
}

#[derive(Clone, Copy, Debug)]
enum Field {
    AmazonNovaApiBase,
    AmazonNovaApiKey,
    GlobalAmazonNovaApiKey,
    MaxCompletionTokens,
    MaxTokens,
    Temperature,
    TopP,
    Tools,
    ReasoningEffort,
    Metadata,
    RejectsUnrelatedKey,
}

#[rstest]
#[case::loads_amazon_nova_api_base(Field::AmazonNovaApiBase)]
#[case::loads_amazon_nova_api_key(Field::AmazonNovaApiKey)]
#[case::loads_global_amazon_nova_api_key(Field::GlobalAmazonNovaApiKey)]
#[case::loads_amazonnovachatconfig_max_completion_tokens(Field::MaxCompletionTokens)]
#[case::loads_amazonnovachatconfig_max_tokens(Field::MaxTokens)]
#[case::loads_amazonnovachatconfig_temperature(Field::Temperature)]
#[case::loads_amazonnovachatconfig_top_p(Field::TopP)]
#[case::loads_amazonnovachatconfig_tools(Field::Tools)]
#[case::loads_amazonnovachatconfig_reasoning_effort(Field::ReasoningEffort)]
#[case::loads_amazonnovachatconfig_metadata(Field::Metadata)]
#[case::rejects_unrelated_key(Field::RejectsUnrelatedKey)]
fn bedrock_nova_chat_config_cases(#[case] field: Field) {
    match field {
        Field::AmazonNovaApiBase => loads_amazon_nova_api_base(),
        Field::AmazonNovaApiKey => loads_amazon_nova_api_key(),
        Field::GlobalAmazonNovaApiKey => loads_global_amazon_nova_api_key(),
        Field::MaxCompletionTokens => loads_amazonnovachatconfig_max_completion_tokens(),
        Field::MaxTokens => loads_amazonnovachatconfig_max_tokens(),
        Field::Temperature => loads_amazonnovachatconfig_temperature(),
        Field::TopP => loads_amazonnovachatconfig_top_p(),
        Field::Tools => loads_amazonnovachatconfig_tools(),
        Field::ReasoningEffort => loads_amazonnovachatconfig_reasoning_effort(),
        Field::Metadata => loads_amazonnovachatconfig_metadata(),
        Field::RejectsUnrelatedKey => rejects_unrelated_key(),
    }
}
