use std::sync::Arc;

use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::apply::{JudgeInputs, OnFailure};
use crate::judge::verdict::EvalInformation;

use super::support::{
    Outcome, RecordingCompletion, guardrail_with, reply_with_score, request_data, texts,
};

#[derive(Debug, Clone, Copy)]
enum Case {
    BelowThresholdLogs,
    AboveThresholdLogs,
    EmptyTextSkipsTheJudge,
    AbsentTextsSkipTheJudge,
}

#[rstest]
#[case::below_threshold_logs(Case::BelowThresholdLogs)]
#[case::above_threshold_logs(Case::AboveThresholdLogs)]
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
            let EvalInformation::Single(information) =
                data.metadata.eval_information.expect("eval information")
            else {
                panic!("one judge leaves one entry");
            };
            assert!(information.passed);
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
