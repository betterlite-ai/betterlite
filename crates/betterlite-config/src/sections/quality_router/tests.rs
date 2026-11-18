#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;
use serde::Deserialize as _;
use serde::de::value::{Error as ValueError, I64Deserializer};
use serde_json::Number;

use super::RawPrice;
use crate::loader::ConfigLoader;
use crate::sections::quality_router::{Deployment, Preferences, RouterConfig, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.quality_router)
        .map_err(|error| error.to_string())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn only(source: &str, variables: &[(&str, &str)]) -> Result<Deployment, String> {
    let mut deployments = settings(source, variables)?.deployments;
    let count = deployments.len();
    if count != 1 {
        return Err(format!("one deployment expected, got {count}"));
    }
    deployments.pop().ok_or_else(|| "no deployment".to_owned())
}

const ROUTER: &str = "\
model_list:
  - model_name: quality-router
    litellm_params:
      model: auto_router/quality_router
      quality_router_config:
        available_models: [fast, strong]
        complexity_to_quality:
          SIMPLE: 1
          REASONING: 4
        default_model: fallback
  - model_name: fast
    litellm_params:
      model: openai/gpt-4o-mini
    model_info:
      litellm_routing_preferences:
        quality_tier: 1
        keywords: [code, Write]
        order: 2
      input_cost_per_token: 0.000001
  - model_name: strong
    litellm_params:
      model: openai/gpt-4o
";

#[test]
fn a_config_without_a_model_list_has_no_deployments() {
    assert_eq!(settings("{}", &[]).unwrap(), Settings::default());
}

#[test]
fn a_deployment_without_the_keys_keeps_its_place() {
    let resolved = settings(
        "model_list:\n  - model_name: plain\n    litellm_params:\n      model: openai/gpt-4o\n",
        &[],
    )
    .unwrap();
    assert_eq!(
        resolved.deployments,
        vec![Deployment {
            model_name: "plain".to_owned(),
            config: None,
            preferences: None,
            input_cost_per_token: None,
        }]
    );
}

#[test]
fn a_full_configuration_resolves_every_key() {
    let resolved = settings(ROUTER, &[]).unwrap();
    assert_eq!(resolved.deployments.len(), 3);
    let router = &resolved.deployments[0];
    assert_eq!(
        router.config,
        Some(RouterConfig {
            available_models: strings(&["fast", "strong"]),
            default_model: Some("fallback".to_owned()),
            complexity_to_quality: BTreeMap::from([
                ("SIMPLE".to_owned(), 1),
                ("REASONING".to_owned(), 4),
            ]),
            extras: BTreeMap::new(),
        })
    );
    let fast = &resolved.deployments[1];
    assert_eq!(
        fast.preferences,
        Some(Preferences {
            quality_tier: 1,
            keywords: strings(&["code", "Write"]),
            order: Some(2),
            extras: BTreeMap::new(),
        })
    );
    assert_eq!(fast.input_cost_per_token, Some(Decimal::new(1, 6)));
    assert_eq!(resolved.deployments[2].preferences, None);
    assert_eq!(resolved.deployments[2].config, None);
}

#[test]
fn the_declared_defaults_fill_the_missing_keys() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      quality_router_config:
        available_models: [fast]
  - model_name: fast
    model_info:
      litellm_routing_preferences:
        quality_tier: 3
";
    let resolved = settings(source, &[]).unwrap();
    let config = resolved.deployments[0].config.as_ref().unwrap();
    assert_eq!(config.default_model, None);
    // absent here, so the pinned default table lands in the field
    assert_eq!(
        config.complexity_to_quality,
        BTreeMap::from([
            ("COMPLEX".to_owned(), 3),
            ("MEDIUM".to_owned(), 2),
            ("REASONING".to_owned(), 4),
            ("SIMPLE".to_owned(), 1),
        ])
    );
    let preferences = resolved.deployments[1].preferences.as_ref().unwrap();
    assert_eq!(preferences.keywords, Vec::<String>::new());
    assert_eq!(preferences.order, None);
}

const ENVIRONMENT: &str = "\
model_list:
  - model_name: router
    litellm_params:
      quality_router_config:
        available_models: [os.environ/FAST]
        complexity_to_quality:
          SIMPLE: os.environ/SIMPLE
        default_model: os.environ/FALLBACK
  - model_name: fast
    model_info:
      litellm_routing_preferences:
        quality_tier: os.environ/TIER
        keywords: [os.environ/KEYWORD]
        order: os.environ/ORDER
      input_cost_per_token: os.environ/COST
";

#[test]
fn environment_references_resolve_once_at_load() {
    let resolved = settings(
        ENVIRONMENT,
        &[
            ("FAST", "fast"),
            ("SIMPLE", "2"),
            ("FALLBACK", "fallback"),
            ("TIER", "3"),
            ("KEYWORD", "code"),
            ("ORDER", "7"),
            ("COST", "0.000002"),
        ],
    )
    .unwrap();
    let config = resolved.deployments[0].config.as_ref().unwrap();
    assert_eq!(config.available_models, strings(&["fast"]));
    assert_eq!(config.default_model, Some("fallback".to_owned()));
    assert_eq!(
        config.complexity_to_quality,
        BTreeMap::from([("SIMPLE".to_owned(), 2)])
    );
    let fast = &resolved.deployments[1];
    let preferences = fast.preferences.as_ref().unwrap();
    assert_eq!(preferences.quality_tier, 3);
    assert_eq!(preferences.keywords, strings(&["code"]));
    assert_eq!(preferences.order, Some(7));
    assert_eq!(fast.input_cost_per_token, Some(Decimal::new(2, 6)));
}

#[test]
fn an_environment_reference_names_a_deployment() {
    let source = "model_list:\n  - model_name: os.environ/FAST\n    model_info:\n      litellm_routing_preferences:\n        quality_tier: 1\n";
    let resolved = settings(source, &[("FAST", "fast")]).unwrap();
    assert_eq!(resolved.deployments[0].model_name, "fast");
}

#[test]
fn an_unquoted_price_keeps_the_digits_the_file_writes() {
    let source = "\
model_list:
  - model_name: priced
    model_info:
      input_cost_per_token: 0.1000000000000000001
";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].input_cost_per_token,
        Some("0.1000000000000000001".parse().unwrap())
    );
}

#[test]
fn a_quoted_price_keeps_the_digits_the_file_writes() {
    let source = "\
model_list:
  - model_name: priced
    model_info:
      input_cost_per_token: \"0.1000000000000000001\"
";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].input_cost_per_token,
        Some("0.1000000000000000001".parse().unwrap())
    );
}

// the schema mode never asks a visitor what it expected, so the message is read through serde's own value deserializer
#[test]
fn a_price_visitor_names_the_scalars_it_accepts() {
    let error = RawPrice::deserialize(I64Deserializer::<ValueError>::new(5)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("a YAML number or an os.environ reference"),
        "{error}"
    );
}

#[test]
fn a_tagged_price_is_read_as_its_number() {
    let source = "\
model_list:
  - model_name: fractional
    model_info:
      input_cost_per_token: !!float 1e-6
  - model_name: whole
    model_info:
      input_cost_per_token: !!int 5
";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].input_cost_per_token,
        Some(Decimal::new(1, 6))
    );
    assert_eq!(
        resolved.deployments[1].input_cost_per_token,
        Some(Decimal::new(5, 0))
    );
}

#[test]
fn an_unquoted_hexadecimal_price_is_read_as_its_integer_value() {
    let source = "\
model_list:
  - model_name: priced
    model_info:
      input_cost_per_token: 0x10
";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
}

#[test]
fn a_tagged_hexadecimal_price_is_read_as_its_integer_value() {
    let source = "\
model_list:
  - model_name: priced
    model_info:
      input_cost_per_token: !!int 0x10
";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
}

#[test]
fn a_separated_hexadecimal_price_is_read_as_its_integer_value() {
    let source = "\
model_list:
  - model_name: priced
    model_info:
      input_cost_per_token: 0x1_0
";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
}

#[test]
fn a_quoted_hexadecimal_price_is_read_as_its_integer_value() {
    let source = "\
model_list:
  - model_name: priced
    model_info:
      input_cost_per_token: \"0x10\"
";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
}

#[test]
fn octal_binary_and_signed_prices_are_read_as_their_integer_values() {
    let source = "\
model_list:
  - model_name: octal
    model_info:
      input_cost_per_token: 0o20
  - model_name: binary
    model_info:
      input_cost_per_token: 0b10000
  - model_name: negative
    model_info:
      input_cost_per_token: -0x10
  - model_name: positive
    model_info:
      input_cost_per_token: +0x10
";
    let resolved = settings(source, &[]).unwrap();
    let deployments = &resolved.deployments;
    assert_eq!(
        deployments[0].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
    assert_eq!(
        deployments[1].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
    assert_eq!(
        deployments[2].input_cost_per_token,
        Some(Decimal::new(-16, 0))
    );
    assert_eq!(
        deployments[3].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
}

#[test]
fn uppercase_radix_prefixes_are_read_as_their_integer_values() {
    let source = "\
model_list:
  - model_name: hex
    model_info:
      input_cost_per_token: 0X10
  - model_name: octal
    model_info:
      input_cost_per_token: 0O20
  - model_name: binary
    model_info:
      input_cost_per_token: 0B10000
";
    let resolved = settings(source, &[]).unwrap();
    let deployments = &resolved.deployments;
    assert_eq!(
        deployments[0].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
    assert_eq!(
        deployments[1].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
    assert_eq!(
        deployments[2].input_cost_per_token,
        Some(Decimal::new(16, 0))
    );
}

#[test]
fn a_price_that_spells_no_number_names_the_key() {
    let source = "\
model_list:
  - model_name: priced
    model_info:
      input_cost_per_token: 0x1.8
";
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("input_cost_per_token"), "{error}");
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      quality_router_config:
        available_models: [fast]
        complexity_to_quality:
          SIMPLE: os.environ/SIMPLE
  - model_name: fast
    model_info:
      litellm_routing_preferences:
        quality_tier: 1
        order: os.environ/ORDER
";
    let resolved = settings(source, &[("SIMPLE", " "), ("ORDER", "")]).unwrap();
    let config = resolved.deployments[0].config.as_ref().unwrap();
    assert_eq!(config.complexity_to_quality, BTreeMap::new());
    assert_eq!(
        resolved.deployments[1].preferences.as_ref().unwrap().order,
        None
    );
}

#[test]
fn an_empty_environment_quality_tier_is_refused() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      quality_router_config:\n        available_models: [fast]\n  - model_name: fast\n    model_info:\n      litellm_routing_preferences:\n        quality_tier: os.environ/TIER\n";
    let error = settings(source, &[("TIER", "")]).unwrap_err();
    assert!(error.contains("quality_tier"), "{error}");
}

#[test]
fn a_missing_environment_reference_names_the_key() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      quality_router_config:
        available_models: [os.environ/ABSENT]
";
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("ABSENT"), "{error}");
    assert!(
        error.contains("litellm_params.quality_router_config.available_models.0"),
        "{error}"
    );
}

#[rstest]
#[case::quoted("quality_tier: \"2\"")]
#[case::boolean("quality_tier: true")]
#[case::fractional("quality_tier: 2.5")]
fn a_wrongly_typed_quality_tier_is_refused(#[case] tier: &str) {
    let source = format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      quality_router_config:\n        available_models: [fast]\n  - model_name: fast\n    model_info:\n      litellm_routing_preferences:\n        {tier}\n"
    );
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("quality_tier"), "{error}");
}

#[test]
fn a_preferences_block_without_a_quality_tier_is_refused() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      quality_router_config:\n        available_models: [fast]\n  - model_name: fast\n    model_info:\n      litellm_routing_preferences:\n        keywords: [code]\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("quality_tier"), "{error}");
}

#[test]
fn an_explicit_null_available_models_list_is_empty() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      quality_router_config:\n        available_models: null\n";
    let resolved = settings(source, &[]).unwrap();
    let config = resolved.deployments[0].config.as_ref().unwrap();
    assert_eq!(config.available_models, Vec::<String>::new());
}

#[test]
fn null_keys_take_the_declared_defaults() {
    let source = "\
model_list:
  - model_name: router
    litellm_params:
      quality_router_config:
        available_models: [fast]
        default_model: null
        complexity_to_quality: null
  - model_name: fast
    model_info:
      input_cost_per_token: null
      litellm_routing_preferences:
        quality_tier: 2
        keywords: null
        order: null
";
    let resolved = settings(source, &[]).unwrap();
    let config = resolved.deployments[0].config.as_ref().unwrap();
    assert_eq!(config.available_models, strings(&["fast"]));
    assert_eq!(config.default_model, None);
    assert_eq!(
        config.complexity_to_quality,
        BTreeMap::from([
            ("COMPLEX".to_owned(), 3),
            ("MEDIUM".to_owned(), 2),
            ("REASONING".to_owned(), 4),
            ("SIMPLE".to_owned(), 1),
        ])
    );
    let fast = &resolved.deployments[1];
    let preferences = fast.preferences.as_ref().unwrap();
    assert_eq!(preferences.quality_tier, 2);
    assert_eq!(preferences.keywords, Vec::<String>::new());
    assert_eq!(preferences.order, None);
    assert_eq!(fast.input_cost_per_token, None);
}

#[test]
fn an_unknown_router_config_key_is_preserved_and_accepted() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      quality_router_config:\n        available_models: [fast]\n        mystery: 1\n";
    let deployment = only(source, &[]).unwrap();
    let config = deployment.config.unwrap();
    assert_eq!(
        config.extras,
        BTreeMap::from([("mystery".to_owned(), Json::Number(Number::from(1_i64)))])
    );
}

#[test]
fn an_unknown_preference_key_is_preserved_and_accepted() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      quality_router_config:\n        available_models: [fast]\n  - model_name: fast\n    model_info:\n      litellm_routing_preferences:\n        quality_tier: 2\n        odd: x\n";
    let resolved = settings(source, &[]).unwrap();
    let preferences = resolved.deployments[1].preferences.as_ref().unwrap();
    assert_eq!(preferences.quality_tier, 2);
    assert_eq!(
        preferences.extras,
        BTreeMap::from([("odd".to_owned(), Json::Text("x".to_owned()))])
    );
}

#[test]
fn the_shipped_valid_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/quality-router-valid.yaml");
    let resolved = settings(source, &[]).unwrap();
    let config = resolved.deployments[0].config.as_ref().unwrap();
    assert_eq!(
        config.available_models,
        strings(&["quality-fast", "quality-strong"])
    );
    let fast = resolved
        .deployments
        .iter()
        .find(|deployment| deployment.model_name == "quality-fast")
        .unwrap();
    let preferences = fast.preferences.as_ref().unwrap();
    assert_eq!(preferences.quality_tier, 1);
    assert_eq!(preferences.keywords, strings(&["code"]));
    assert_eq!(fast.input_cost_per_token, Some(Decimal::new(1, 6)));
}

#[test]
fn the_shipped_invalid_seed_is_refused() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/quality-router-invalid.yaml");
    let error = settings(source, &[]).unwrap_err();
    assert!(!error.is_empty());
}
