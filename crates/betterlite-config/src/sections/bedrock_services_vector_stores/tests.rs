#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::settings::JsonValue;

use super::{Settings, UserContext};

fn yaml_pairs(pairs: &[(&str, &str)]) -> String {
    let mut text = String::new();
    for (key, value) in pairs {
        writeln!(text, "    {key}: {value}").unwrap();
    }
    text
}

fn load(
    params: &[(&str, &str)],
    settings: &[(&str, &str)],
    environment: &[(&str, &str)],
) -> Result<Settings, ConfigError> {
    let deployment = if params.is_empty() {
        "  litellm_params: {}\n".to_owned()
    } else {
        format!("  litellm_params:\n{}", yaml_pairs(params))
    };
    let global = if settings.is_empty() {
        "litellm_settings: {}\n".to_owned()
    } else {
        format!("litellm_settings:\n{}", yaml_pairs(settings))
    };
    let source = format!("model_list:\n- model_name: oracle\n{deployment}{global}");
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(&source)
        .map(|config| config.sections.bedrock_services_vector_stores)
}

fn value(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn deployed(settings: &Settings) -> UserContext {
    settings
        .deployments
        .first()
        .expect("one deployment")
        .clone()
}

#[rstest]
#[case::loads_usercontext(
    &[("userContext", "{userId: tenant-a}")],
    &[],
    &[],
    Some(r#"{"userId":"tenant-a"}"#),
    None
)]
#[case::loads_user_context(
    &[("user_context", "{userId: tenant-b}")],
    &[],
    &[],
    None,
    Some(r#"{"userId":"tenant-b"}"#)
)]
#[case::loads_usercontext_reference(
    &[("userContext", "{userId: os.environ/USER_CONTEXT_ID}")],
    &[],
    &[("USER_CONTEXT_ID", "referenced-user")],
    Some(r#"{"userId":"referenced-user"}"#),
    None
)]
#[case::loads_both_forms_without_merging(
    &[
        ("userContext", "{userId: camel-user}"),
        ("user_context", "{userId: snake-user}"),
    ],
    &[],
    &[],
    Some(r#"{"userId":"camel-user"}"#),
    Some(r#"{"userId":"snake-user"}"#)
)]
#[case::defaults_usercontext(&[], &[], &[], None, None)]
fn knowledge_base_config_cases(
    #[case] params: &[(&str, &str)],
    #[case] settings: &[(&str, &str)],
    #[case] environment: &[(&str, &str)],
    #[case] camel: Option<&str>,
    #[case] snake: Option<&str>,
) {
    let loaded = load(params, settings, environment).expect("the config should load");
    let deployment = deployed(&loaded);
    assert_eq!(deployment.camel, camel.map(value));
    assert_eq!(deployment.snake, snake.map(value));
    assert!(loaded.global.camel.is_none());
    assert!(loaded.global.snake.is_none());
}

#[rstest]
#[case::loads_global_usercontext(
    &[("userContext", "{userId: global-user}")],
    Some(r#"{"userId":"global-user"}"#),
    None
)]
#[case::loads_global_user_context(
    &[("user_context", "{userId: global-reader}")],
    None,
    Some(r#"{"userId":"global-reader"}"#)
)]
#[case::defaults_global_usercontext(&[], None, None)]
fn global_knowledge_base_config_cases(
    #[case] settings: &[(&str, &str)],
    #[case] camel: Option<&str>,
    #[case] snake: Option<&str>,
) {
    let loaded = load(&[], settings, &[]).expect("the config should load");
    assert_eq!(loaded.global.camel, camel.map(value));
    assert_eq!(loaded.global.snake, snake.map(value));
    assert_eq!(loaded.deployments.len(), 1);
    assert_eq!(loaded.deployments[0], UserContext::default());
}

#[test]
fn a_structured_user_context_is_preserved() {
    let loaded = load(
        &[(
            "userContext",
            "{userId: tenant-a, filters: [1, 2], nested: {enabled: true}}",
        )],
        &[],
        &[],
    )
    .expect("the config should load");
    assert_eq!(
        deployed(&loaded).camel,
        Some(value(
            r#"{"userId":"tenant-a","filters":[1,2],"nested":{"enabled":true}}"#
        ))
    );
}

#[test]
fn a_missing_environment_reference_names_the_variable_and_its_field() {
    let error = load(
        &[("userContext", "{userId: os.environ/MISSING_USER_CONTEXT}")],
        &[],
        &[],
    )
    .expect_err("the reference cannot resolve");
    let rendered = error.to_string();
    assert!(rendered.contains("MISSING_USER_CONTEXT"));
    assert!(rendered.contains("model_list.0.litellm_params.userContext.userId"));
}

#[test]
fn an_unrelated_deployment_key_is_rejected() {
    let error = load(&[("userContexts", "1")], &[], &[]).expect_err("the key is unknown");
    let ConfigError::Unsupported { paths } = error else {
        panic!("expected unsupported keys, got {error:?}");
    };
    assert_eq!(paths, ["model_list.0.litellm_params.userContexts"]);
}

#[test]
fn an_unrelated_global_key_is_rejected() {
    let error = load(&[], &[("user_contexts", "1")], &[]).expect_err("the key is unknown");
    let ConfigError::Unsupported { paths } = error else {
        panic!("expected unsupported keys, got {error:?}");
    };
    assert_eq!(paths, ["litellm_settings.user_contexts"]);
}

#[test]
fn knowledge_base_corpus_seeds_load() {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let configured =
        include_str!("../../../fuzz/corpus/config_yaml/knowledge_base_user_context.yaml");
    let configured_config = loader.load(configured).expect("the seed loads");
    let configured_settings = configured_config.sections.bedrock_services_vector_stores;
    assert_eq!(
        deployed(&configured_settings).camel,
        Some(value(r#"{"userId":"tenant-a"}"#))
    );
    assert_eq!(
        configured_settings.global.snake,
        Some(value(r#"{"userId":"global-reader"}"#))
    );
    let defaults = include_str!("../../../fuzz/corpus/config_yaml/knowledge_base_defaults.yaml");
    let default_config = loader.load(defaults).expect("the seed loads");
    let default_settings = default_config.sections.bedrock_services_vector_stores;
    assert_eq!(default_settings.deployments.len(), 1);
    assert_eq!(default_settings.deployments[0], UserContext::default());
    assert_eq!(default_settings.global, UserContext::default());
}
