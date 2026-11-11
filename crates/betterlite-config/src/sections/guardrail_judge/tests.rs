#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{CriterionWeight, EventHookParameter, GuardrailMode, Settings, TagValue};

const JUDGE_ENTRY: &str = "guardrails:\n  - guardrail_name: quality\n    litellm_params:\n      guardrail: llm_as_a_judge\n      judge_model: gpt-4o-mini\n      criteria:\n        - {name: accuracy, description: is it right, weight: 60}\n        - {name: safety, description: is it safe, weight: 40}\n      overall_threshold: 85\n      on_failure: log\n      mode: pre_call\n      default_on: true\n";

const MIXED_GUARDRAILS: &str = "guardrails:\n  - guardrail_name: other\n    litellm_params:\n      guardrail: not_a_guardrail\n  - guardrail_name: quality\n    litellm_params:\n      guardrail: llm_as_a_judge\n      judge_model: gpt-4o-mini\n";

const ENVIRONMENT_THRESHOLD: &str = "guardrails:\n  - guardrail_name: quality\n    litellm_params:\n      guardrail: llm_as_a_judge\n      overall_threshold: os.environ/JUDGE_THRESHOLD\n";

const MODE_SHAPES: &str = "guardrails:\n  - guardrail_name: list\n    litellm_params:\n      guardrail: llm_as_a_judge\n      criteria:\n        - {name: a, weight: 50.0}\n      mode: [pre_call, post_call]\n  - guardrail_name: tagged\n    litellm_params:\n      guardrail: llm_as_a_judge\n      mode:\n        tags: {judge: [pre_call]}\n        default: post_call\n";

const NON_NUMERIC_WEIGHT: &str = "guardrails:\n  - guardrail_name: quality\n    litellm_params:\n      guardrail: llm_as_a_judge\n      criteria:\n        - {name: a, weight: heavy}\n";

const UNKNOWN_KEY: &str = "guardrails:\n  - guardrail_name: quality\n    litellm_params:\n      guardrail: llm_as_a_judge\n      judge_model_name: gpt-4o-mini\n";

#[derive(Debug, Clone, Copy)]
enum Expectation {
    EveryField,
    OtherGuardrailSkipped,
    EnvironmentThreshold,
    MissingEnvironment,
    FloatWeightAndModes,
    NonNumericWeight,
    UnknownKey,
}

fn lookups(expectation: Expectation) -> BTreeMap<String, String> {
    match expectation {
        Expectation::EnvironmentThreshold => {
            BTreeMap::from([("JUDGE_THRESHOLD".to_owned(), "72.5".to_owned())])
        }
        Expectation::EveryField
        | Expectation::OtherGuardrailSkipped
        | Expectation::MissingEnvironment
        | Expectation::FloatWeightAndModes
        | Expectation::NonNumericWeight
        | Expectation::UnknownKey => BTreeMap::new(),
    }
}

fn load(source: &str, expectation: Expectation) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: lookups(expectation),
    }
    .load(source)
    .map(|config| config.sections.guardrail_judge)
}

#[rstest]
#[case::every_field(JUDGE_ENTRY, Expectation::EveryField)]
#[case::other_guardrail_skipped(MIXED_GUARDRAILS, Expectation::OtherGuardrailSkipped)]
#[case::environment_threshold(ENVIRONMENT_THRESHOLD, Expectation::EnvironmentThreshold)]
#[case::missing_environment(ENVIRONMENT_THRESHOLD, Expectation::MissingEnvironment)]
#[case::float_weight_and_modes(MODE_SHAPES, Expectation::FloatWeightAndModes)]
#[case::non_numeric_weight(NON_NUMERIC_WEIGHT, Expectation::NonNumericWeight)]
#[case::unknown_key(UNKNOWN_KEY, Expectation::UnknownKey)]
fn judge_config_tests_behavior(#[case] source: &str, #[case] expectation: Expectation) {
    match expectation {
        Expectation::EveryField => {
            let settings = load(source, expectation).expect("config");
            let configured = settings.guardrails.first().expect("guardrail");
            assert_eq!(configured.guardrail_name.as_deref(), Some("quality"));
            assert_eq!(
                configured.params.judge_model.as_deref(),
                Some("gpt-4o-mini")
            );
            assert_eq!(configured.params.overall_threshold, Some(85.0_f64));
            assert_eq!(configured.params.on_failure.as_deref(), Some("log"));
            assert_eq!(configured.params.default_on, Some(true));
            assert_eq!(
                configured.params.mode,
                Some(EventHookParameter::Single("pre_call".to_owned()))
            );
            let criteria = configured.params.criteria.as_ref().expect("criteria");
            assert_eq!(criteria.len(), 2);
            assert_eq!(criteria[0].name, "accuracy");
            assert_eq!(criteria[0].description, "is it right");
            assert!(matches!(criteria[0].weight, CriterionWeight::Integer(60)));
            assert_eq!(criteria[0].weight.to_prompt_text(), "60");
            assert_eq!(
                CriterionWeight::Integer(60).to_decimal(),
                Some(rust_decimal::Decimal::from(60_i64))
            );
            assert_eq!(
                CriterionWeight::Float(60.0).to_decimal(),
                Some(rust_decimal::Decimal::from(60_i64))
            );
        }
        Expectation::OtherGuardrailSkipped => {
            let settings = load(source, expectation).expect("config");
            assert_eq!(settings.guardrails.len(), 1);
            let configured = settings.guardrails.first().expect("guardrail");
            assert_eq!(configured.guardrail_name.as_deref(), Some("quality"));
            assert_eq!(configured.params.overall_threshold, None);
            assert_eq!(configured.params.mode, None);
            assert_eq!(configured.params.default_on, None);
        }
        Expectation::EnvironmentThreshold => {
            let settings = load(source, expectation).expect("config");
            assert_eq!(
                settings
                    .guardrails
                    .first()
                    .expect("guardrail")
                    .params
                    .overall_threshold,
                Some(72.5_f64)
            );
        }
        Expectation::MissingEnvironment => {
            let error = load(source, expectation).expect_err("missing variable");
            assert!(
                matches!(
                    &error,
                    ConfigError::MissingEnvironment { variable, .. }
                        if variable == "JUDGE_THRESHOLD"
                ),
                "{error:?}"
            );
        }
        Expectation::FloatWeightAndModes => {
            let settings = load(source, expectation).expect("config");
            assert_eq!(settings.guardrails.len(), 2);
            let first = settings.guardrails.first().expect("guardrail");
            assert_eq!(
                first.params.mode,
                Some(EventHookParameter::Many(vec![
                    "pre_call".to_owned(),
                    "post_call".to_owned()
                ]))
            );
            let weight = first.params.criteria.as_ref().expect("criteria")[0].weight;
            assert!(matches!(weight, CriterionWeight::Float(50.0_f64)));
            assert_eq!(weight.to_prompt_text(), "50.0");
            let tagged = settings
                .guardrails
                .get(1)
                .expect("tagged guardrail")
                .params
                .mode
                .clone()
                .expect("mode");
            let EventHookParameter::Mode(mode) = tagged else {
                panic!("expected a tagged mode");
            };
            assert_eq!(
                mode,
                GuardrailMode {
                    tags: BTreeMap::from([(
                        "judge".to_owned(),
                        TagValue::Many(vec!["pre_call".to_owned()])
                    )]),
                    default: Some(TagValue::One("post_call".to_owned())),
                }
            );
            assert_eq!(mode.hook_names(), vec!["pre_call", "post_call"]);
        }
        Expectation::NonNumericWeight => {
            let error = load(source, expectation).expect_err("non-numeric weight");
            assert!(matches!(&error, ConfigError::Yaml { .. }), "{error:?}");
        }
        Expectation::UnknownKey => {
            let error = load(source, expectation).expect_err("unknown key");
            assert!(
                matches!(
                    &error,
                    ConfigError::Unsupported { paths }
                        if paths.iter().any(|path| path.contains("judge_model_name"))
                ),
                "{error:?}"
            );
        }
    }
}
