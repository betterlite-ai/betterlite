use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::{
    Criterion, CriterionWeight, EventHookParameter, GuardrailMode, JudgeParams, TagValue,
};
use rstest::rstest;

use crate::judge::config::{GuardrailEntry, LitellmParamsSource, ParamKey, get_param, parse_param};
use crate::judge::error::JudgeError;
use crate::judge::hooks::{EventHookConfig, GuardrailEventHook, coerce_event_hook};

use super::support::{criterion, params_with_model};

#[derive(Debug, Clone, Copy)]
enum Case {
    TypedAttributeWins,
    MappingKey,
    ParsedAttribute,
    AbsentEverywhere,
    WrongType,
    TypedFloatThreshold,
    TypedFloatWeights,
    IntegerThreshold,
    FloatThreshold,
    TextThreshold,
    RawCriteria,
    RawMode,
    RawDefaultOn,
    CoerceDefault,
    CoerceSingle,
    CoerceMany,
    CoerceTagged,
    CoerceUnknown,
    ParseEveryHook,
}

fn value_map(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn mapping_entry(mapping: &BTreeMap<String, GuardrailValue>) -> GuardrailEntry<'_> {
    GuardrailEntry {
        guardrail_name: Some("judge"),
        litellm_params: Some(LitellmParamsSource::Mapping(mapping)),
    }
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(value_map(entries))
}

fn criterion_value(name: &str, weight: GuardrailValue, description: &str) -> GuardrailValue {
    object(&[
        ("name", GuardrailValue::Text(name.to_owned())),
        ("description", GuardrailValue::Text(description.to_owned())),
        ("weight", weight),
    ])
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

#[rstest]
#[case::typed_attribute_wins(Case::TypedAttributeWins)]
#[case::mapping_key(Case::MappingKey)]
#[case::parsed_attribute(Case::ParsedAttribute)]
#[case::absent_everywhere(Case::AbsentEverywhere)]
#[case::wrong_type(Case::WrongType)]
#[case::typed_float_threshold(Case::TypedFloatThreshold)]
#[case::typed_float_weights(Case::TypedFloatWeights)]
#[case::integer_threshold(Case::IntegerThreshold)]
#[case::float_threshold(Case::FloatThreshold)]
#[case::text_threshold(Case::TextThreshold)]
#[case::raw_criteria(Case::RawCriteria)]
#[case::raw_mode(Case::RawMode)]
#[case::raw_default_on(Case::RawDefaultOn)]
#[case::coerce_default(Case::CoerceDefault)]
#[case::coerce_single(Case::CoerceSingle)]
#[case::coerce_many(Case::CoerceMany)]
#[case::coerce_tagged(Case::CoerceTagged)]
#[case::coerce_unknown(Case::CoerceUnknown)]
#[case::parse_every_hook(Case::ParseEveryHook)]
fn judge_config_tests_behavior(#[case] case: Case) {
    match case {
        Case::TypedAttributeWins => {
            let params = params_with_model(Some("typed".to_owned()));
            let mapping = value_map(&[("judge_model", text("raw"))]);
            let resolved = get_param(&params, &mapping_entry(&mapping), ParamKey::JudgeModel);
            assert_eq!(resolved, Some(GuardrailValue::Text("typed".to_owned())));
        }
        Case::MappingKey => {
            let params = JudgeParams::default();
            let mapping = value_map(&[("judge_model", text("raw"))]);
            let resolved = get_param(&params, &mapping_entry(&mapping), ParamKey::JudgeModel);
            assert_eq!(resolved, Some(GuardrailValue::Text("raw".to_owned())));
        }
        Case::ParsedAttribute => {
            let params = JudgeParams::default();
            let parsed = params_with_model(Some("parsed".to_owned()));
            let entry = GuardrailEntry {
                guardrail_name: Some("judge"),
                litellm_params: Some(LitellmParamsSource::Parsed(&parsed)),
            };
            let resolved = get_param(&params, &entry, ParamKey::JudgeModel);
            assert_eq!(resolved, Some(GuardrailValue::Text("parsed".to_owned())));
        }
        Case::AbsentEverywhere => {
            let params = JudgeParams::default();
            let mapping = BTreeMap::new();
            let resolved = get_param(&params, &mapping_entry(&mapping), ParamKey::JudgeModel);
            assert_eq!(resolved, None);
        }
        Case::WrongType => {
            let params = JudgeParams::default();
            let mapping = value_map(&[("judge_model", GuardrailValue::Boolean(true))]);
            let error =
                parse_param::<String>(&params, &mapping_entry(&mapping), ParamKey::JudgeModel)
                    .expect_err("wrong type");
            assert!(matches!(
                error,
                JudgeError::InvalidParam {
                    key: "judge_model",
                    ..
                }
            ));
        }
        Case::TypedFloatThreshold => {
            let params = JudgeParams {
                judge_model: None,
                criteria: None,
                overall_threshold: Some(72.5_f64),
                on_failure: None,
                mode: None,
                default_on: None,
            };
            let mapping = BTreeMap::new();
            let resolved = parse_param::<f64>(
                &params,
                &mapping_entry(&mapping),
                ParamKey::OverallThreshold,
            )
            .expect("param");
            assert_eq!(resolved.map(f64::to_bits), Some(72.5_f64.to_bits()));
        }
        Case::TypedFloatWeights => {
            let params = JudgeParams {
                judge_model: None,
                criteria: Some(vec![
                    criterion("a", CriterionWeight::Integer(50), "first"),
                    criterion("b", CriterionWeight::Float(50.0), "second"),
                ]),
                overall_threshold: None,
                on_failure: None,
                mode: None,
                default_on: None,
            };
            let mapping = BTreeMap::new();
            let resolved = parse_param::<Vec<Criterion>>(
                &params,
                &mapping_entry(&mapping),
                ParamKey::Criteria,
            )
            .expect("param");
            assert_eq!(
                resolved,
                Some(vec![
                    Criterion {
                        name: "a".to_owned(),
                        description: "first".to_owned(),
                        weight: CriterionWeight::Integer(50),
                    },
                    Criterion {
                        name: "b".to_owned(),
                        description: "second".to_owned(),
                        weight: CriterionWeight::Float(50.0),
                    },
                ])
            );
        }
        Case::IntegerThreshold => {
            let params = JudgeParams::default();
            let mapping = value_map(&[("overall_threshold", GuardrailValue::SignedInteger(90))]);
            let resolved = parse_param::<f64>(
                &params,
                &mapping_entry(&mapping),
                ParamKey::OverallThreshold,
            )
            .expect("param");
            assert_eq!(
                resolved.map(f64::to_bits),
                Some(90.0_f64.to_bits()),
                "an integer threshold becomes a float"
            );
        }
        Case::FloatThreshold => {
            let params = JudgeParams::default();
            let mapping = value_map(&[("overall_threshold", GuardrailValue::Decimal(72.5))]);
            let resolved = parse_param::<f64>(
                &params,
                &mapping_entry(&mapping),
                ParamKey::OverallThreshold,
            )
            .expect("param");
            assert_eq!(resolved.map(f64::to_bits), Some(72.5_f64.to_bits()));
        }
        Case::TextThreshold => {
            let params = JudgeParams::default();
            let mapping =
                value_map(&[("overall_threshold", GuardrailValue::Text("80".to_owned()))]);
            let error = parse_param::<f64>(
                &params,
                &mapping_entry(&mapping),
                ParamKey::OverallThreshold,
            )
            .expect_err("text is not a threshold");
            assert!(matches!(
                error,
                JudgeError::InvalidParam {
                    key: "overall_threshold",
                    ..
                }
            ));
        }
        Case::RawCriteria => {
            let params = JudgeParams::default();
            let mapping = value_map(&[(
                "criteria",
                GuardrailValue::Array(vec![
                    criterion_value("a", GuardrailValue::SignedInteger(50), "first"),
                    criterion_value("b", GuardrailValue::Decimal(50.0), "second"),
                ]),
            )]);
            let resolved = parse_param::<Vec<Criterion>>(
                &params,
                &mapping_entry(&mapping),
                ParamKey::Criteria,
            )
            .expect("param");
            assert_eq!(
                resolved,
                Some(vec![
                    Criterion {
                        name: "a".to_owned(),
                        description: "first".to_owned(),
                        weight: CriterionWeight::Integer(50),
                    },
                    Criterion {
                        name: "b".to_owned(),
                        description: "second".to_owned(),
                        weight: CriterionWeight::Float(50.0),
                    },
                ])
            );
        }
        Case::RawMode => {
            let params = JudgeParams::default();
            let mapping = value_map(&[("mode", text("pre_call"))]);
            let resolved = parse_param::<EventHookParameter>(
                &params,
                &mapping_entry(&mapping),
                ParamKey::Mode,
            )
            .expect("param");
            assert_eq!(
                resolved,
                Some(EventHookParameter::Single("pre_call".to_owned()))
            );
        }
        Case::RawDefaultOn => {
            let params = JudgeParams::default();
            let mapping = value_map(&[("default_on", GuardrailValue::Boolean(true))]);
            let resolved =
                parse_param::<bool>(&params, &mapping_entry(&mapping), ParamKey::DefaultOn)
                    .expect("param");
            assert_eq!(resolved, Some(true));
        }
        Case::CoerceDefault => {
            assert_eq!(
                coerce_event_hook(None).expect("coerce"),
                EventHookConfig::Single(GuardrailEventHook::PostCall)
            );
        }
        Case::CoerceSingle => {
            let parameter = EventHookParameter::Single("pre_call".to_owned());
            assert_eq!(
                coerce_event_hook(Some(&parameter)).expect("coerce"),
                EventHookConfig::Single(GuardrailEventHook::PreCall)
            );
        }
        Case::CoerceMany => {
            let parameter =
                EventHookParameter::Many(vec!["pre_call".to_owned(), "post_call".to_owned()]);
            assert_eq!(
                coerce_event_hook(Some(&parameter)).expect("coerce"),
                EventHookConfig::Many(vec![
                    GuardrailEventHook::PreCall,
                    GuardrailEventHook::PostCall,
                ])
            );
        }
        Case::CoerceTagged => {
            let mode = GuardrailMode {
                tags: BTreeMap::from([(
                    "judge".to_owned(),
                    TagValue::Many(vec!["pre_call".to_owned()]),
                )]),
                default: Some(TagValue::One("post_call".to_owned())),
            };
            let parameter = EventHookParameter::Mode(mode.clone());
            assert_eq!(
                coerce_event_hook(Some(&parameter)).expect("coerce"),
                EventHookConfig::Mode(mode)
            );
        }
        Case::CoerceUnknown => {
            let parameter = EventHookParameter::Single("sometimes".to_owned());
            let error = coerce_event_hook(Some(&parameter)).expect_err("unknown hook");
            assert!(matches!(
                error,
                JudgeError::InvalidEventHook { value } if value == "sometimes"
            ));
        }
        Case::ParseEveryHook => {
            let hooks = [
                ("pre_call", GuardrailEventHook::PreCall),
                ("post_call", GuardrailEventHook::PostCall),
                ("during_call", GuardrailEventHook::DuringCall),
                ("logging_only", GuardrailEventHook::LoggingOnly),
                ("pre_mcp_call", GuardrailEventHook::PreMcpCall),
                ("during_mcp_call", GuardrailEventHook::DuringMcpCall),
                ("post_mcp_call", GuardrailEventHook::PostMcpCall),
                (
                    "realtime_input_transcription",
                    GuardrailEventHook::RealtimeInputTranscription,
                ),
            ];
            for (name, hook) in hooks {
                assert_eq!(
                    GuardrailEventHook::parse(name).expect("known hook"),
                    hook,
                    "{name}"
                );
                assert_eq!(hook.as_str(), name);
            }
            GuardrailEventHook::parse("sometimes").expect_err("unknown hook");
        }
    }
}
