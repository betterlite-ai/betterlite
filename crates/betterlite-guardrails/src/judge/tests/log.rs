use std::sync::Arc;

use rstest::rstest;

use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::apply::{JudgeInputs, OnFailure};
use crate::judge::hooks::{EventHookConfig, GuardrailEventHook};
use crate::judge::verdict::EvalInformation;

use super::support::{
    Outcome, RecordingCompletion, guardrail_with, only_record, reply_with_score, request_data,
    texts,
};

#[derive(Debug, Clone, Copy)]
enum Case {
    BelowThresholdLogs,
    AboveThresholdLogs,
    RecordNamesTheEventType,
    RecordKeepsAmbiguousMode,
    EmptyTextSkipsTheJudge,
    AbsentTextsSkipTheJudge,
}

#[rstest]
#[case::below_threshold_logs(Case::BelowThresholdLogs)]
#[case::above_threshold_logs(Case::AboveThresholdLogs)]
#[case::record_names_the_event_type(Case::RecordNamesTheEventType)]
#[case::record_keeps_ambiguous_mode(Case::RecordKeepsAmbiguousMode)]
#[case::empty_text_skips_the_judge(Case::EmptyTextSkipsTheJudge)]
#[case::absent_texts_skip_the_judge(Case::AbsentTextsSkipTheJudge)]
#[tokio::test]
async fn judge_log_tests_behavior(#[case] case: Case) {
    match case {
        Case::BelowThresholdLogs => {
            let completion = RecordingCompletion::new(Outcome::Reply(Some(reply_with_score(50.0))));
            let mut guardrail = guardrail_with(Arc::clone(&completion));
            guardrail.on_failure = OnFailure::Log;
            let mut data = request_data(Vec::new());
            let returned = guardrail
                .apply_guardrail(
                    texts(&["bad response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("log mode never blocks");
            assert_eq!(returned.texts, Some(vec!["bad response".to_owned()]));
            assert_eq!(
                only_record(&data).guardrail_status,
                GuardrailStatus::GuardrailIntervened
            );
            let EvalInformation::Single(information) =
                data.metadata.eval_information.expect("eval information")
            else {
                panic!("one judge leaves one entry");
            };
            assert!(!information.passed);
            assert_eq!(completion.call_count(), 1);
        }
        Case::AboveThresholdLogs => {
            let mut guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            guardrail.on_failure = OnFailure::Log;
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["good response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("above the threshold passes");
            assert_eq!(
                only_record(&data).guardrail_status,
                GuardrailStatus::Success
            );
            let EvalInformation::Single(information) =
                data.metadata.eval_information.expect("eval information")
            else {
                panic!("one judge leaves one entry");
            };
            assert!(information.passed);
        }
        Case::RecordNamesTheEventType => {
            let mut guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            guardrail.event_hook = EventHookConfig::Single(GuardrailEventHook::PreCall);
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(texts(&["ask"]), &mut data, GuardrailInputType::Request)
                .await
                .expect("above the threshold passes");
            assert_eq!(
                only_record(&data).guardrail_mode,
                EventHookConfig::Single(GuardrailEventHook::PreCall)
            );
        }
        Case::RecordKeepsAmbiguousMode => {
            let mut guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            let configured = EventHookConfig::Many(vec![
                GuardrailEventHook::PreCall,
                GuardrailEventHook::DuringCall,
            ]);
            guardrail.event_hook = configured.clone();
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(texts(&["ask"]), &mut data, GuardrailInputType::Request)
                .await
                .expect("above the threshold passes");
            assert_eq!(only_record(&data).guardrail_mode, configured);
        }
        Case::EmptyTextSkipsTheJudge => {
            let completion = RecordingCompletion::new(Outcome::Failure);
            let guardrail = guardrail_with(Arc::clone(&completion));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(texts(&[]), &mut data, GuardrailInputType::Response)
                .await
                .expect("no text passes through");
            assert_eq!(completion.call_count(), 0);
            assert_eq!(data.metadata.eval_information, None);
            assert!(data.metadata.guardrail_records.is_empty());
        }
        Case::AbsentTextsSkipTheJudge => {
            let completion = RecordingCompletion::new(Outcome::Failure);
            let guardrail = guardrail_with(Arc::clone(&completion));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    JudgeInputs::default(),
                    &mut data,
                    GuardrailInputType::Request,
                )
                .await
                .expect("an empty request passes through");
            assert_eq!(completion.call_count(), 0);
            assert_eq!(data.metadata.eval_information, None);
        }
    }
}
