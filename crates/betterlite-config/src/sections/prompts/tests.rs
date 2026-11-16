#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::Section;
use crate::sections::prompts::{Raw, Settings};

fn loaded_with(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.prompts)
}

fn loaded(source: &str) -> Result<Settings, ConfigError> {
    loaded_with(source, &[])
}

fn settings(source: &str) -> Settings {
    loaded(source).expect("the prompts settings load")
}

fn settings_with(source: &str, environment: &[(&str, &str)]) -> Settings {
    loaded_with(source, environment).expect("the prompts settings load")
}

fn nested_entry(levels: usize, leaf: Json) -> IndexMap<String, Json> {
    let mut inner = leaf;
    for _ in 0..levels {
        let mut wrapper = IndexMap::new();
        wrapper.insert("nested".to_owned(), inner);
        inner = Json::Object(wrapper);
    }
    let mut params = IndexMap::new();
    params.insert("extra".to_owned(), inner);
    let mut entry = IndexMap::new();
    entry.insert("prompt_id".to_owned(), Json::Text("deep".to_owned()));
    entry.insert("litellm_params".to_owned(), Json::Object(params));
    entry
}

fn converted(entry: IndexMap<String, Json>, environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Raw {
        prompts: Some(Json::List(vec![Json::Object(entry)])),
    }
    .convert(&lookup)
    .expect("the entry converts")
}

fn entry_leaf(settings: &Settings) -> &Json {
    let mut current = &settings.prompts[0]["litellm_params"]["extra"];
    while let Json::Object(mapping) = current {
        let Some((_key, inner)) = mapping.first() else {
            panic!("the fixture holds a leaf");
        };
        current = inner;
    }
    current
}

#[test]
fn an_absent_prompts_key_reads_as_an_empty_list() {
    assert!(settings("model_list: []\n").prompts.is_empty());
}

#[test]
fn a_null_prompts_key_reads_as_an_empty_list() {
    assert!(settings("prompts:\n").prompts.is_empty());
}

#[test]
fn an_empty_prompts_list_reads_as_an_empty_list() {
    assert!(settings("prompts: []\n").prompts.is_empty());
}

#[test]
fn entries_keep_their_order_and_their_raw_fields() {
    let settings = settings(
        "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      prompt_integration: dotprompt\n      dotprompt_content: Hello\n    future_option: true\n  - prompt_id: farewell\n    litellm_params:\n      prompt_integration: gitlab\n",
    );
    assert_eq!(settings.prompts.len(), 2);
    let expected: IndexMap<String, Json> = serde_json::from_str(
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt","dotprompt_content":"Hello"},"future_option":true}"#,
    )
    .expect("the expected entry is a mapping");
    assert_eq!(settings.prompts[0], expected);
    assert_eq!(
        settings.prompts[0]
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["prompt_id", "litellm_params", "future_option"]
    );
    assert_eq!(
        settings.prompts[1].get("prompt_id"),
        Some(&Json::Text("farewell".to_owned()))
    );
}

#[rstest]
#[case::text("prompts: not-a-list\n")]
#[case::mapping("prompts:\n  prompt_id: greeting\n")]
#[case::number("prompts: 5\n")]
fn a_prompts_value_that_is_not_a_list_is_refused_and_names_the_key(#[case] source: &str) {
    let error = loaded(source).expect_err("the prompts value is not a list");
    assert!(matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "prompts"));
    assert!(error.to_string().contains("prompts"), "{error}");
}

#[rstest]
#[case::text("prompts:\n  - not-a-mapping\n")]
#[case::number("prompts:\n  - 5\n")]
#[case::list("prompts:\n  - [a, b]\n")]
#[case::null("prompts:\n  - null\n")]
fn an_entry_that_is_not_a_mapping_is_refused_and_names_its_index(#[case] source: &str) {
    let error = loaded(source).expect_err("the entry is not a mapping");
    assert!(matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "prompts[0]"));
    assert!(error.to_string().contains("prompts[0]"), "{error}");
}

#[test]
fn a_bad_entry_is_named_by_its_position() {
    let source = "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      prompt_integration: dotprompt\n  - not-a-mapping\n";
    let error = loaded(source).expect_err("the second entry is not a mapping");
    assert!(matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "prompts[1]"));
}

#[test]
fn environment_references_in_an_entry_are_resolved() {
    let source = "prompts:\n  - prompt_id: greeting\n    api_key: os.environ/PROMPT_KEY\n    api_base: os.environ/PROMPT_BASE\n    litellm_params:\n      prompt_integration: gitlab\n      gitlab_config:\n        access_token: os.environ/GITLAB_TOKEN\n";
    let settings = settings_with(
        source,
        &[
            ("PROMPT_KEY", "sk-test-key"),
            ("PROMPT_BASE", "https://gitlab.example.test"),
            ("GITLAB_TOKEN", "glpat-token"),
        ],
    );
    let entry = &settings.prompts[0];
    assert_eq!(
        entry.get("api_key"),
        Some(&Json::Text("sk-test-key".to_owned()))
    );
    assert_eq!(
        entry.get("api_base"),
        Some(&Json::Text("https://gitlab.example.test".to_owned()))
    );
    let Some(Json::Object(params)) = entry.get("litellm_params") else {
        panic!("litellm_params is a mapping");
    };
    let Some(Json::Object(gitlab)) = params.get("gitlab_config") else {
        panic!("gitlab_config is a mapping");
    };
    assert_eq!(
        gitlab.get("access_token"),
        Some(&Json::Text("glpat-token".to_owned()))
    );
}

#[test]
fn a_missing_variable_becomes_null() {
    let source = "prompts:\n  - prompt_id: greeting\n    api_key: os.environ/ABSENT\n";
    let settings = settings_with(source, &[]);
    assert_eq!(settings.prompts[0].get("api_key"), Some(&Json::Null));
}

#[test]
fn an_empty_variable_becomes_null() {
    let source = "prompts:\n  - prompt_id: greeting\n    api_key: os.environ/EMPTY\n";
    let settings = settings_with(source, &[("EMPTY", "")]);
    assert_eq!(settings.prompts[0].get("api_key"), Some(&Json::Null));
}

#[rstest]
#[case::true_value("true", Json::Bool(true))]
#[case::false_value("false", Json::Bool(false))]
#[case::empty_value("", Json::Bool(false))]
#[case::short_true("y", Json::Bool(true))]
#[case::upper_short_true(" T ", Json::Bool(true))]
#[case::short_false("n", Json::Bool(false))]
#[case::upper_short_false("F", Json::Bool(false))]
#[case::on_value("on", Json::Bool(true))]
#[case::zero_value("0", Json::Bool(false))]
fn a_flag_reference_in_litellm_params_reads_as_a_bool(#[case] value: &str, #[case] expected: Json) {
    let source = "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      prompt_integration: dotprompt\n      ignore_prompt_manager_model: os.environ/IGNORE_MODEL\n      ignore_prompt_manager_optional_params: os.environ/IGNORE_PARAMS\n";
    let settings = settings_with(
        source,
        &[("IGNORE_MODEL", value), ("IGNORE_PARAMS", "false")],
    );
    let Some(Json::Object(params)) = settings.prompts[0].get("litellm_params") else {
        panic!("litellm_params is a mapping");
    };
    assert_eq!(params.get("ignore_prompt_manager_model"), Some(&expected));
    assert_eq!(
        params.get("ignore_prompt_manager_optional_params"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn a_missing_flag_variable_reads_as_false() {
    let source = "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      ignore_prompt_manager_model: os.environ/ABSENT\n";
    let settings = settings_with(source, &[]);
    let Some(Json::Object(params)) = settings.prompts[0].get("litellm_params") else {
        panic!("litellm_params is a mapping");
    };
    assert_eq!(
        params.get("ignore_prompt_manager_model"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn a_flag_variable_that_is_not_a_bool_is_a_load_error_naming_it() {
    let source = "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      ignore_prompt_manager_optional_params: os.environ/IGNORE_PARAMS\n";
    let error = loaded_with(source, &[("IGNORE_PARAMS", "maybe")]).expect_err("maybe is no bool");
    assert!(matches!(&error, ConfigError::Environment { .. }));
    assert!(error.to_string().contains("IGNORE_PARAMS"), "{error}");
    assert!(!error.to_string().contains("maybe"), "{error}");
}

#[rstest]
#[case::two("2", Json::Number(2.into()))]
#[case::negative("-3", Json::Number((-3).into()))]
#[case::empty("", Json::Null)]
fn a_version_reference_reads_as_a_number(#[case] value: &str, #[case] expected: Json) {
    let source = "prompts:\n  - prompt_id: greeting\n    version: os.environ/PROMPT_VERSION\n";
    let settings = settings_with(source, &[("PROMPT_VERSION", value)]);
    assert_eq!(settings.prompts[0].get("version"), Some(&expected));
}

#[test]
fn a_missing_version_variable_becomes_null() {
    let source = "prompts:\n  - prompt_id: greeting\n    version: os.environ/ABSENT\n";
    let settings = settings_with(source, &[]);
    assert_eq!(settings.prompts[0].get("version"), Some(&Json::Null));
}

#[test]
fn a_version_variable_that_is_not_a_number_is_a_load_error_naming_it() {
    let source = "prompts:\n  - prompt_id: greeting\n    version: os.environ/PROMPT_VERSION\n";
    let error =
        loaded_with(source, &[("PROMPT_VERSION", "latest")]).expect_err("latest is no number");
    assert!(matches!(&error, ConfigError::Environment { .. }));
    assert!(error.to_string().contains("PROMPT_VERSION"), "{error}");
    assert!(!error.to_string().contains("latest"), "{error}");
}

#[test]
fn a_flag_key_outside_litellm_params_keeps_its_text() {
    let source = "prompts:\n  - prompt_id: greeting\n    ignore_prompt_manager_model: os.environ/IGNORE_MODEL\n    litellm_params:\n      version: os.environ/PROMPT_VERSION\n";
    let settings = settings_with(source, &[("IGNORE_MODEL", "true"), ("PROMPT_VERSION", "2")]);
    assert_eq!(
        settings.prompts[0].get("ignore_prompt_manager_model"),
        Some(&Json::Text("true".to_owned()))
    );
    let Some(Json::Object(params)) = settings.prompts[0].get("litellm_params") else {
        panic!("litellm_params is a mapping");
    };
    assert_eq!(params.get("version"), Some(&Json::Text("2".to_owned())));
}

#[test]
fn a_string_that_only_contains_the_prefix_later_is_left_as_written() {
    let source = "prompts:\n  - prompt_id: greeting\n    api_base: https://example.test/os.environ/PROMPT_BASE\n";
    let settings = settings_with(source, &[("PROMPT_BASE", "https://gitlab.example.test")]);
    assert_eq!(
        settings.prompts[0].get("api_base"),
        Some(&Json::Text(
            "https://example.test/os.environ/PROMPT_BASE".to_owned()
        ))
    );
}

#[test]
fn mapping_items_inside_a_list_are_resolved_and_plain_items_are_left() {
    let source = "prompts:\n  - prompt_id: greeting\n    params:\n      - os.environ/PLAIN\n      - api_key: os.environ/LIST_KEY\n";
    let settings = settings_with(source, &[("PLAIN", "kept"), ("LIST_KEY", "list-key")]);
    let Some(Json::List(items)) = settings.prompts[0].get("params") else {
        panic!("params is a list");
    };
    assert_eq!(items[0], Json::Text("os.environ/PLAIN".to_owned()));
    let Some(Json::Object(item)) = items.get(1) else {
        panic!("the second item is a mapping");
    };
    assert_eq!(
        item.get("api_key"),
        Some(&Json::Text("list-key".to_owned()))
    );
}

#[rstest]
#[case::last_resolved_level(98, Json::Text("deep-value".to_owned()))]
#[case::first_skipped_level(99, Json::Text("os.environ/DEEP".to_owned()))]
fn the_depth_limit_counts_the_prompts_entry_and_its_litellm_params(
    #[case] levels: usize,
    #[case] expected: Json,
) {
    let entry = nested_entry(levels, Json::Text("os.environ/DEEP".to_owned()));
    let settings = converted(entry, &[("DEEP", "deep-value")]);
    assert_eq!(entry_leaf(&settings), &expected);
}
