use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::JudgeParams;
use reqwest::StatusCode;
use rstest::rstest;

use crate::judge::apply::{LLMAsAJudgeGuardrail, OnFailure};
use crate::judge::config::{GuardrailEntry, LitellmParamsSource};
use crate::judge::error::JudgeError;
use crate::judge::hooks::{EventHookConfig, GuardrailEventHook};
use crate::judge::initialize::{DEFAULT_THRESHOLD, initialize_guardrail};

use super::support::{
    Outcome, RecordingCompletion, RecordingRegistry, entry_from, params_with_model, valid_mapping,
};

#[derive(Debug, Clone, Copy)]
enum Case {
    ValidRegisters,
    MissingName,
    EmptyName,
    MissingJudgeModel,
    EmptyJudgeModel,
    MissingCriteria,
    EmptyCriteria,
    WeightBelowTolerance,
    WeightAboveTolerance,
    WeightAtLowTolerance,
    WeightAtHighTolerance,
    InvalidOnFailure,
    UnknownMode,
    TypedParamsWin,
    DefaultsWhenAbsent,
}

fn with_weights(weights: &[GuardrailValue]) -> BTreeMap<String, GuardrailValue> {
    let criteria = weights
        .iter()
        .enumerate()
        .map(|(index, weight)| {
            GuardrailValue::Object(BTreeMap::from([
                (
                    "name".to_owned(),
                    GuardrailValue::Text(format!("criterion-{index}")),
                ),
                (
                    "description".to_owned(),
                    GuardrailValue::Text("checked".to_owned()),
                ),
                ("weight".to_owned(), weight.clone()),
            ]))
        })
        .collect();
    let mut mapping = valid_mapping();
    mapping.insert("criteria".to_owned(), GuardrailValue::Array(criteria));
    mapping
}

struct Initialized {
    result: Result<Arc<LLMAsAJudgeGuardrail<io::Error>>, JudgeError>,
    registered: usize,
}

fn initialize(params: &JudgeParams, mapping: &BTreeMap<String, GuardrailValue>) -> Initialized {
    let mut registry = RecordingRegistry::default();
    let result = initialize_guardrail(
        params,
        &entry_from(mapping),
        RecordingCompletion::new(Outcome::NoChoices),
        &mut registry,
    );
    Initialized {
        result,
        registered: registry.registered.len(),
    }
}

#[rstest]
#[case::valid_registers(Case::ValidRegisters)]
#[case::missing_name(Case::MissingName)]
#[case::empty_name(Case::EmptyName)]
#[case::missing_judge_model(Case::MissingJudgeModel)]
#[case::empty_judge_model(Case::EmptyJudgeModel)]
#[case::missing_criteria(Case::MissingCriteria)]
#[case::empty_criteria(Case::EmptyCriteria)]
#[case::weight_below_tolerance(Case::WeightBelowTolerance)]
#[case::weight_above_tolerance(Case::WeightAboveTolerance)]
#[case::weight_at_low_tolerance(Case::WeightAtLowTolerance)]
#[case::weight_at_high_tolerance(Case::WeightAtHighTolerance)]
#[case::invalid_on_failure(Case::InvalidOnFailure)]
#[case::unknown_mode(Case::UnknownMode)]
#[case::typed_params_win(Case::TypedParamsWin)]
#[case::defaults_when_absent(Case::DefaultsWhenAbsent)]
fn judge_initialize_tests_behavior(#[case] case: Case) {
    let params = JudgeParams::default();
    match case {
        Case::ValidRegisters => {
            let mapping = valid_mapping();
            let Initialized { result, registered } = initialize(&params, &mapping);
            let guardrail = result.expect("valid config");
            assert_eq!(registered, 1);
            assert_eq!(
                guardrail.overall_threshold.to_bits(),
                DEFAULT_THRESHOLD.to_bits()
            );
            assert_eq!(guardrail.on_failure, OnFailure::Block);
            assert!(!guardrail.default_on);
            assert_eq!(
                guardrail.event_hook,
                EventHookConfig::Single(GuardrailEventHook::PostCall)
            );
        }
        Case::MissingName => {
            let mapping = valid_mapping();
            let mut registry = RecordingRegistry::default();
            let entry = GuardrailEntry {
                guardrail_name: None,
                litellm_params: Some(LitellmParamsSource::Mapping(&mapping)),
            };
            let error = initialize_guardrail(
                &params,
                &entry,
                RecordingCompletion::new(Outcome::NoChoices),
                &mut registry,
            )
            .err()
            .expect("a name is required");
            assert!(matches!(error, JudgeError::MissingGuardrailName));
            assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
            assert!(registry.registered.is_empty());
        }
        Case::EmptyName => {
            let mapping = valid_mapping();
            let mut registry = RecordingRegistry::default();
            let entry = GuardrailEntry {
                guardrail_name: Some(""),
                litellm_params: Some(LitellmParamsSource::Mapping(&mapping)),
            };
            let error = initialize_guardrail(
                &params,
                &entry,
                RecordingCompletion::new(Outcome::NoChoices),
                &mut registry,
            )
            .err()
            .expect("an empty name is rejected like a missing one");
            assert!(matches!(error, JudgeError::MissingGuardrailName));
            assert!(registry.registered.is_empty());
        }
        Case::MissingJudgeModel => {
            let mut mapping = valid_mapping();
            mapping.remove("judge_model");
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error.err().expect("judge_model is required"),
                JudgeError::MissingJudgeModel
            ));
        }
        Case::EmptyJudgeModel => {
            let mut mapping = valid_mapping();
            mapping.insert(
                "judge_model".to_owned(),
                GuardrailValue::Text(String::new()),
            );
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error
                    .err()
                    .expect("an empty judge_model is rejected like a missing one"),
                JudgeError::MissingJudgeModel
            ));
        }
        Case::MissingCriteria => {
            let mut mapping = valid_mapping();
            mapping.remove("criteria");
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error.err().expect("criteria is required"),
                JudgeError::MissingCriteria
            ));
        }
        Case::EmptyCriteria => {
            let mapping = with_weights(&[]);
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error.err().expect("criteria cannot be empty"),
                JudgeError::MissingCriteria
            ));
        }
        Case::WeightBelowTolerance => {
            let mapping = with_weights(&[GuardrailValue::Decimal(99.4)]);
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error.err().expect("99.4 is outside the tolerance"),
                JudgeError::WeightSum { .. }
            ));
        }
        Case::WeightAboveTolerance => {
            let mapping = with_weights(&[GuardrailValue::Decimal(100.6)]);
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error.err().expect("100.6 is outside the tolerance"),
                JudgeError::WeightSum { .. }
            ));
        }
        Case::WeightAtLowTolerance => {
            let mapping = with_weights(&[GuardrailValue::Decimal(99.5)]);
            let Initialized { result, registered } = initialize(&params, &mapping);
            result.expect("valid weights");
            assert_eq!(registered, 1);
        }
        Case::WeightAtHighTolerance => {
            let mapping =
                with_weights(&[GuardrailValue::Decimal(50.0), GuardrailValue::Decimal(50.5)]);
            let Initialized { result, registered } = initialize(&params, &mapping);
            result.expect("valid weights");
            assert_eq!(registered, 1);
        }
        Case::InvalidOnFailure => {
            let mut mapping = valid_mapping();
            mapping.insert(
                "on_failure".to_owned(),
                GuardrailValue::Text("explode".to_owned()),
            );
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error.err().expect("on_failure is validated"),
                JudgeError::InvalidOnFailure { value } if value == "explode"
            ));
        }
        Case::UnknownMode => {
            let mut mapping = valid_mapping();
            mapping.insert(
                "mode".to_owned(),
                GuardrailValue::Text("sometimes".to_owned()),
            );
            let error = initialize(&params, &mapping).result;
            assert!(matches!(
                error.err().expect("unknown hooks are rejected"),
                JudgeError::InvalidEventHook { value } if value == "sometimes"
            ));
        }
        Case::TypedParamsWin => {
            let typed = params_with_model(Some("typed-judge".to_owned()));
            let mapping = valid_mapping();
            let result = initialize(&typed, &mapping).result;
            let guardrail = result.expect("valid config");
            assert_eq!(guardrail.judge_model, "typed-judge");
        }
        Case::DefaultsWhenAbsent => {
            let mut mapping = valid_mapping();
            mapping.remove("overall_threshold");
            mapping.remove("on_failure");
            let Initialized { result, registered } = initialize(&params, &mapping);
            let guardrail = result.expect("the defaults fill in");
            assert_eq!(registered, 1);
            assert_eq!(guardrail.overall_threshold.to_bits(), 80.0_f64.to_bits());
            assert_eq!(guardrail.on_failure, OnFailure::Block);
        }
    }
}
