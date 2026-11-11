use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::{CriterionWeight, JudgeParams};
use rstest::rstest;

use crate::judge::apply::OnFailure;
use crate::judge::error::JudgeError;
use crate::judge::hooks::{EventHookConfig, GuardrailEventHook, SUPPORTED_EVENT_HOOKS};
use crate::judge::initialize::initialize_guardrail;

use super::support::{Outcome, RecordingCompletion, RecordingRegistry, entry_from, valid_mapping};

#[derive(Debug, Clone, Copy)]
enum Case {
    SupportedHooks,
    StoresConfiguredFields,
    AcceptsLoggingOnly,
    RejectsUnsupportedHook,
}

#[rstest]
#[case::supported_hooks(Case::SupportedHooks)]
#[case::stores_configured_fields(Case::StoresConfiguredFields)]
#[case::accepts_logging_only(Case::AcceptsLoggingOnly)]
#[case::rejects_unsupported_hook(Case::RejectsUnsupportedHook)]
fn judge_construct_tests_behavior(#[case] case: Case) {
    match case {
        Case::SupportedHooks => {
            assert_eq!(
                SUPPORTED_EVENT_HOOKS.map(GuardrailEventHook::as_str),
                ["pre_call", "during_call", "post_call"]
            );
        }
        Case::StoresConfiguredFields => {
            let mut mapping = valid_mapping();
            mapping.insert(
                "overall_threshold".to_owned(),
                GuardrailValue::SignedInteger(70),
            );
            mapping.insert(
                "on_failure".to_owned(),
                GuardrailValue::Text("log".to_owned()),
            );
            mapping.insert(
                "mode".to_owned(),
                GuardrailValue::Text("during_call".to_owned()),
            );
            mapping.insert("default_on".to_owned(), GuardrailValue::Boolean(true));
            let completion = RecordingCompletion::new(Outcome::NoChoices);
            let mut registry = RecordingRegistry::default();
            let guardrail = initialize_guardrail(
                &JudgeParams::default(),
                &entry_from(&mapping),
                completion,
                &mut registry,
            )
            .expect("initialize");
            assert_eq!(guardrail.guardrail_name, "judge");
            assert_eq!(guardrail.judge_model, "gpt-4o-mini");
            assert_eq!(guardrail.criteria.len(), 2);
            assert_eq!(guardrail.criteria[0].weight, CriterionWeight::Integer(60));
            assert_eq!(guardrail.overall_threshold.to_bits(), 70.0_f64.to_bits());
            assert_eq!(guardrail.on_failure, OnFailure::Log);
            assert_eq!(
                guardrail.event_hook,
                EventHookConfig::Single(GuardrailEventHook::DuringCall)
            );
            assert!(guardrail.default_on);
            assert_eq!(registry.registered.len(), 1);
        }
        Case::AcceptsLoggingOnly => {
            let mut mapping = valid_mapping();
            mapping.insert(
                "mode".to_owned(),
                GuardrailValue::Text("logging_only".to_owned()),
            );
            let mut registry = RecordingRegistry::default();
            let guardrail = initialize_guardrail(
                &JudgeParams::default(),
                &entry_from(&mapping),
                RecordingCompletion::new(Outcome::NoChoices),
                &mut registry,
            )
            .expect("logging_only is allowed for apply_guardrail guardrails");
            assert_eq!(
                guardrail.event_hook,
                EventHookConfig::Single(GuardrailEventHook::LoggingOnly)
            );
        }
        Case::RejectsUnsupportedHook => {
            let mut mapping = valid_mapping();
            mapping.insert(
                "mode".to_owned(),
                GuardrailValue::Text("pre_mcp_call".to_owned()),
            );
            let mut registry = RecordingRegistry::default();
            let error = initialize_guardrail(
                &JudgeParams::default(),
                &entry_from(&mapping),
                RecordingCompletion::new(Outcome::NoChoices),
                &mut registry,
            )
            .err()
            .expect("mcp hooks are not supported");
            assert!(matches!(
                error,
                JudgeError::UnsupportedEventHook {
                    hook: GuardrailEventHook::PreMcpCall
                }
            ));
            assert!(registry.registered.is_empty());
        }
    }
}
