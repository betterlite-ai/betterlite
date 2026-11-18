#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::loader::ConfigLoader;
use crate::sections::Section;
use crate::sections::parse_section;
use crate::sections::tier_definitions::{
    Config, Raw, Settings, TierDefinition, TierEntry, TierModelEntry, TierParam, TierPool,
};

fn parse(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let mut ignored = Vec::new();
    let raw = parse_section::<Raw>(source, &mut ignored).map_err(|error| error.to_string())?;
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Section::convert(*raw, &lookup).map_err(|error| error.to_string())
}

// one deployment whose complexity router config holds the given keys, already indented under it
fn deployment(keys: &str) -> String {
    format!(
        "model_list:\n  - model_name: primary\n    litellm_params:\n      complexity_router_config:\n{keys}"
    )
}

fn only(keys: &str, variables: &[(&str, &str)]) -> Result<Option<Config>, String> {
    let mut deployments = parse(&deployment(keys), variables)?.deployments;
    let count = deployments.len();
    if count != 1 {
        return Err(format!("one deployment expected, got {count}"));
    }
    Ok(deployments.pop().flatten())
}

fn object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn entry(model_name: &str, params: &[(&str, Json)]) -> TierEntry {
    TierEntry::Detailed(TierModelEntry {
        model_name: model_name.to_owned(),
        litellm_params: params
            .iter()
            .map(|(key, value)| ((*key).to_owned(), param(value)))
            .collect(),
    })
}

fn param(value: &Json) -> TierParam {
    serde_json::from_value(serde_json::to_value(value).expect("the parameter serializes"))
        .expect("the parameter deserializes")
}

fn model(name: &str) -> TierEntry {
    TierEntry::Model(name.to_owned())
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

#[test]
fn a_config_without_a_model_list_has_no_deployments() {
    assert_eq!(parse("{}", &[]).unwrap(), Settings::default());
    assert_eq!(parse("model_list: []", &[]).unwrap(), Settings::default());
}

#[test]
fn a_deployment_without_the_router_config_carries_none() {
    let source = "model_list:\n  - model_name: primary\n    litellm_params:\n      model: gpt-4o\n";
    assert_eq!(parse(source, &[]).unwrap().deployments, [None]);
}

#[test]
fn deployments_keep_their_model_list_positions() {
    let source = "model_list:
  - model_name: plain
    litellm_params:
      model: gpt-4o
  - model_name: custom
    litellm_params:
      complexity_router_config:
        non_reasoning_tier: true
";
    let deployments = parse(source, &[]).unwrap().deployments;
    assert_eq!(deployments.len(), 2);
    assert_eq!(deployments[0], None);
    assert!(deployments[1].as_ref().unwrap().non_reasoning_tier);
}

#[test]
fn all_four_keys_resolve_to_their_config_shape() {
    let keys = "        tier_definitions:
          - name: ECO
            description: cheap asks
        non_reasoning_tier: true
        tier_labels:
          SIMPLE: Cheap
          MEDIUM: Standard
        tier_models:
          ECO: eco-mini
          STANDARD: [std-a, std-b]
";
    let config = only(keys, &[]).unwrap().unwrap();
    assert_eq!(
        config.tier_definitions,
        Some(vec![TierDefinition {
            name: "ECO".to_owned(),
            description: Some("cheap asks".to_owned()),
        }])
    );
    assert!(config.non_reasoning_tier);
    assert_eq!(
        config.tier_labels.get("SIMPLE").map(String::as_str),
        Some("Cheap")
    );
    assert_eq!(
        config.tier_labels.get("MEDIUM").map(String::as_str),
        Some("Standard")
    );
    let pools = config.tier_models.unwrap();
    assert_eq!(&pools["ECO"], &TierPool::Single(model("eco-mini")));
    assert_eq!(
        &pools["STANDARD"],
        &TierPool::Pool(vec![model("std-a"), model("std-b")])
    );
}

#[test]
fn absent_pools_stay_absent_and_an_explicit_empty_map_is_kept() {
    let absent = only("        tier_labels:\n          SIMPLE: Cheap\n", &[])
        .unwrap()
        .unwrap();
    assert_eq!(absent.tier_models, None);
    let empty = only("        tier_models: {}\n", &[]).unwrap().unwrap();
    assert_eq!(empty.tier_models, Some(BTreeMap::new()));
}

#[test]
fn null_keys_take_their_defaults() {
    let keys = "        tier_definitions: null
        tier_labels: null
        tier_models: null
";
    assert_eq!(only(keys, &[]).unwrap().unwrap(), Config::default());
}

#[test]
fn a_null_pool_entry_is_dropped() {
    let config = only("        tier_models:\n          SIMPLE: null\n", &[])
        .unwrap()
        .unwrap();
    assert!(config.tier_models.unwrap().is_empty());
}

#[test]
fn a_single_object_entry_is_normalized_with_its_params() {
    let keys = "        tier_models:
          REASONING:
            model_name: opus
            litellm_params:
              thinking: {type: enabled}
";
    let config = only(keys, &[]).unwrap().unwrap();
    let pools = config.tier_models.unwrap();
    let expected = entry(
        "opus",
        &[("thinking", object(&[("type", text("enabled"))]))],
    );
    assert_eq!(&pools["REASONING"], &TierPool::Single(expected));
}

#[test]
fn a_pool_mixes_bare_names_and_parameterized_entries() {
    let keys = "        tier_models:
          REASONING:
            - model_name: opus
              litellm_params: {reasoning_effort: xhigh}
            - abc
";
    let config = only(keys, &[]).unwrap().unwrap();
    let pools = config.tier_models.unwrap();
    let expected = TierPool::Pool(vec![
        entry("opus", &[("reasoning_effort", text("xhigh"))]),
        model("abc"),
    ]);
    assert_eq!(&pools["REASONING"], &expected);
}

#[test]
fn liteellm_key_names_are_accepted_as_aliases() {
    let keys = "        tiers:
          SIMPLE: small
        enable_non_reasoning_tier: true
";
    let config = only(keys, &[]).unwrap().unwrap();
    assert!(config.non_reasoning_tier);
    assert_eq!(
        &config.tier_models.unwrap()["SIMPLE"],
        &TierPool::Single(model("small"))
    );
}

#[test]
fn references_resolve_once_from_the_environment() {
    let keys = "        tier_definitions:
          - name: os.environ/TIER_NAME
            description: os.environ/TIER_TEXT
        non_reasoning_tier: os.environ/FLAG
        tier_labels:
          SIMPLE: os.environ/LABEL
        tier_models:
          ECO:
            model_name: os.environ/MODEL
            litellm_params:
              temperature: os.environ/TEMPERATURE
              extra: {nested: os.environ/NESTED}
              items: [{key: os.environ/IN_LIST_OBJECT}, os.environ/LITERAL]
";
    let variables = [
        ("TIER_NAME", "ECO"),
        ("TIER_TEXT", "cheap asks"),
        ("FLAG", "true"),
        ("LABEL", "Cheap"),
        ("MODEL", "eco-mini"),
        ("TEMPERATURE", "0.2"),
        ("NESTED", "nested-value"),
        ("IN_LIST_OBJECT", "object-value"),
        ("LITERAL", "literal-value"),
    ];
    let config = only(keys, &variables).unwrap().unwrap();
    assert_eq!(
        config.tier_definitions.unwrap()[0],
        TierDefinition {
            name: "ECO".to_owned(),
            description: Some("cheap asks".to_owned()),
        }
    );
    assert!(config.non_reasoning_tier);
    assert_eq!(config.tier_labels["SIMPLE"], "Cheap");
    let expected = entry(
        "eco-mini",
        &[
            ("temperature", text("0.2")),
            ("extra", object(&[("nested", text("nested-value"))])),
            (
                "items",
                Json::List(vec![
                    object(&[("key", text("object-value"))]),
                    text("literal-value"),
                ]),
            ),
        ],
    );
    assert_eq!(
        &config.tier_models.unwrap()["ECO"],
        &TierPool::Single(expected)
    );
}

#[test]
fn a_bare_model_inside_a_list_stays_literal() {
    let config = only(
        "        tier_models:\n          SIMPLE: [os.environ/MODEL]\n",
        &[("MODEL", "small")],
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        &config.tier_models.unwrap()["SIMPLE"],
        &TierPool::Pool(vec![model("os.environ/MODEL")])
    );
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let keys = "        tier_definitions:
          - name: ECO
            description: os.environ/EMPTY
        non_reasoning_tier: os.environ/FLAG
        tier_labels:
          SIMPLE: os.environ/EMPTY
        tier_models:
          ECO:
            - os.environ/EMPTY
            - kept
";
    let config = only(keys, &[("EMPTY", ""), ("FLAG", "")]).unwrap().unwrap();
    assert_eq!(config.tier_definitions.unwrap()[0].description, None);
    assert!(!config.non_reasoning_tier);
    assert!(config.tier_labels.is_empty());
    assert_eq!(
        &config.tier_models.unwrap()["ECO"],
        &TierPool::Pool(vec![model("os.environ/EMPTY"), model("kept")])
    );
}

#[test]
fn an_explicit_empty_label_stays_as_written() {
    let config = only("        tier_labels:\n          SIMPLE: \"\"\n", &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        config.tier_labels.get("SIMPLE").map(String::as_str),
        Some("")
    );
}

#[test]
fn an_empty_variable_model_name_fails_the_load() {
    let keys = "        tier_models:
          ECO:
            model_name: os.environ/EMPTY
";
    let error = only(keys, &[("EMPTY", "")]).unwrap_err();
    assert!(error.contains("EMPTY"), "{error}");
    assert!(error.contains("model_name"), "{error}");
}

#[test]
fn an_unset_variable_fails_the_load_with_its_name_and_path() {
    let error = only(
        "        tier_labels:\n          SIMPLE: os.environ/LABEL\n",
        &[],
    )
    .unwrap_err();
    assert!(error.contains("LABEL"), "{error}");
    assert!(error.contains("tier_labels.SIMPLE"), "{error}");
}

#[test]
fn a_parameter_reference_is_resolved_too() {
    let keys = "        tier_models:
          SIMPLE:
            model_name: small
            litellm_params: {temperature: os.environ/TEMPERATURE}
";
    let error = only(keys, &[]).unwrap_err();
    assert!(error.contains("TEMPERATURE"), "{error}");
}

#[test]
fn the_loader_accepts_the_task_keys_as_supported() {
    let source = "model_list:
  - model_name: primary
    litellm_params:
      model: gpt-4o
      complexity_router_config:
        tier_definitions:
          - name: ECO
            description: cheap asks
        non_reasoning_tier: false
        tier_labels:
          SIMPLE: Cheap
        tier_models:
          ECO: [eco-a, eco-b]
";
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    let deployment = config.sections.tier_definitions.deployments[0]
        .as_ref()
        .unwrap();
    assert_eq!(deployment.tier_definitions.as_ref().unwrap()[0].name, "ECO");
    assert_eq!(deployment.tier_labels["SIMPLE"], "Cheap");
    assert_eq!(
        &deployment.tier_models.as_ref().unwrap()["ECO"],
        &TierPool::Pool(vec![model("eco-a"), model("eco-b")])
    );
}

#[test]
fn a_number_in_a_text_slot_is_a_load_error() {
    assert!(only("        tier_labels:\n          SIMPLE: 3\n", &[]).is_err());
}

#[test]
fn a_parameter_entry_without_a_model_name_is_a_load_error() {
    let keys =
        "        tier_models:\n          SIMPLE:\n            litellm_params: {temperature: 0.2}\n";
    assert!(only(keys, &[]).is_err());
}

#[test]
fn a_parameter_entry_with_non_mapping_params_is_a_load_error() {
    let keys = "        tier_models:\n          SIMPLE:\n            model_name: small\n            litellm_params: []\n";
    assert!(only(keys, &[]).is_err());
}

#[test]
fn an_unknown_field_inside_a_parameter_entry_is_ignored() {
    let keys = "        tier_models:
          SIMPLE:
            model_name: small
            unknown: 1
";
    let config = only(keys, &[]).unwrap().unwrap();
    assert_eq!(
        &config.tier_models.unwrap()["SIMPLE"],
        &TierPool::Single(model("small"))
    );
}

#[test]
fn the_separate_tier_model_configs_map_is_not_read() {
    let keys = "        tiers:
          REASONING: opus
        tier_model_configs:
          REASONING:
            - {model_name: opus, litellm_params: {reasoning_effort: xhigh}}
";
    let config = only(keys, &[]).unwrap().unwrap();
    assert_eq!(
        &config.tier_models.unwrap()["REASONING"],
        &TierPool::Single(model("opus"))
    );
}

#[test]
fn an_object_entry_fails_the_loader_until_the_baseline_view_accepts_it() {
    let source = "model_list:
  - model_name: primary
    litellm_params:
      complexity_router_config:
        tier_models:
          SIMPLE:
            model_name: small
            litellm_params: {temperature: 0.2}
";
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("a model name or a list of model names"),
        "{error}"
    );
}

#[test]
fn a_number_parameter_survives_as_a_number() {
    let keys = "        tier_models:
          SIMPLE:
            model_name: small
            litellm_params: {max_tokens: 512}
";
    let config = only(keys, &[]).unwrap().unwrap();
    let expected = entry("small", &[("max_tokens", number(512))]);
    assert_eq!(
        &config.tier_models.unwrap()["SIMPLE"],
        &TierPool::Single(expected)
    );
}

#[test]
fn absent_null_and_empty_parameters_are_one_empty_map() {
    let absent = only(
        "        tier_models:\n          SIMPLE:\n            model_name: small\n",
        &[],
    )
    .unwrap()
    .unwrap();
    let empty = only(
        "        tier_models:\n          SIMPLE:\n            model_name: small\n            litellm_params: {}\n",
        &[],
    )
    .unwrap()
    .unwrap();
    let null = only(
        "        tier_models:\n          SIMPLE:\n            model_name: small\n            litellm_params: null\n",
        &[],
    )
    .unwrap()
    .unwrap();
    assert_eq!(absent, empty);
    assert_eq!(empty, null);
    assert_eq!(
        absent.tier_models.unwrap()["SIMPLE"],
        TierPool::Single(entry("small", &[]))
    );
}

#[test]
fn parameter_text_equality_compares_every_byte() {
    let first = entry("small", &[("api_key", text("secret-keys"))]);
    let same = entry("small", &[("api_key", text("secret-keys"))]);
    let swapped = entry("small", &[("api_key", text("secret-kept"))]);
    let longer = entry("small", &[("api_key", text("secret-keys-and-more"))]);
    assert_eq!(first, same);
    assert_ne!(first, swapped);
    assert_ne!(first, longer);
}

#[test]
fn parameter_values_serialize_in_the_shape_the_file_spells() {
    let keys = "        tier_models:
          SIMPLE:
            model_name: small
            litellm_params:
              api_key: sk-secret
              max_tokens: 512
              thinking: {type: enabled}
              stop: [END, STOP]
";
    let config = only(keys, &[]).unwrap().unwrap();
    let rendered = serde_json::to_string(&config).unwrap();
    assert!(rendered.contains(r#""api_key":"sk-secret""#), "{rendered}");
    assert!(rendered.contains(r#""max_tokens":512"#), "{rendered}");
    assert!(
        rendered.contains(r#""thinking":{"type":"enabled"}"#),
        "{rendered}"
    );
    assert!(rendered.contains(r#""stop":["END","STOP"]"#), "{rendered}");
}

#[test]
fn a_parameter_map_exposes_its_entries() {
    let keys = "        tier_models:
          SIMPLE:
            model_name: small
            litellm_params: {max_tokens: 512, top_p: 1}
";
    let config = only(keys, &[]).unwrap().unwrap();
    let pools = config.tier_models.unwrap();
    let TierPool::Single(TierEntry::Detailed(entry)) = &pools["SIMPLE"] else {
        panic!("a parameterized entry expected");
    };
    assert_eq!(entry.litellm_params.len(), 2);
    assert!(!entry.litellm_params.is_empty());
    assert_eq!(
        entry
            .litellm_params
            .get("max_tokens")
            .map(|param| Json::from(param.clone())),
        Some(number(512))
    );
    let absent = only(
        "        tier_models:\n          SIMPLE:\n            model_name: small\n",
        &[],
    )
    .unwrap()
    .unwrap();
    let pools = absent.tier_models.unwrap();
    let TierPool::Single(TierEntry::Detailed(entry)) = &pools["SIMPLE"] else {
        panic!("a parameterized entry expected");
    };
    assert!(entry.litellm_params.is_empty());
}
