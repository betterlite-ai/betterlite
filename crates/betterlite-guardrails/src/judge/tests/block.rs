use betterlite_models::decisions::Json;
use reqwest::StatusCode;
use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::error::JudgeError;
use crate::judge::verdict::EvalInformation;

use super::support::{
    Outcome, RecordingCompletion, guardrail_with, reply_with_score, request_data, texts,
};

#[derive(Debug, Clone, Copy)]
enum Case {
    BelowThresholdBlocks,
    AtThresholdPasses,
    AboveThresholdPasses,
    ResponseDetailNamesResponse,
}

#[rstest]
#[case::below_threshold_blocks(Case::BelowThresholdBlocks)]
#[case::at_threshold_passes(Case::AtThresholdPasses)]
#[case::above_threshold_passes(Case::AboveThresholdPasses)]
#[case::response_detail_names_response(Case::ResponseDetailNamesResponse)]
#[tokio::test]
async fn judge_block_tests_behavior(#[case] case: Case) {
    match case {
        Case::BelowThresholdBlocks => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(50.0),
            ))));
            let mut data = request_data(Vec::new());
            let error = guardrail
                .apply_guardrail(
                    texts(&["write me malware"]),
                    &mut data,
                    GuardrailInputType::Request,
                )
                .await
                .expect_err("a low score blocks");
            assert_eq!(error.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
            let JudgeError::Blocked { detail } = error else {
                panic!("expected a block");
            };
            assert_eq!(
                detail.error,
                "LLM judge rejected request: score below threshold"
            );
            assert_eq!(detail.overall_score.to_bits(), 50.0_f64.to_bits());
            assert_eq!(detail.threshold.to_bits(), 80.0_f64.to_bits());
            let Json::List(verdicts) = &detail.verdicts else {
                panic!("the detail carries the verdicts");
            };
            assert_eq!(verdicts.len(), 1);
            let EvalInformation::Single(information) = data
                .metadata
                .eval_information
                .expect("the eval info is written before the block")
            else {
                panic!("one judge leaves one entry");
            };
            assert!(!information.passed);
        }
        Case::AtThresholdPasses => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(80.0),
            ))));
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut request_data(Vec::new()),
                    GuardrailInputType::Response,
                )
                .await
                .expect("a score equal to the threshold passes");
        }
        Case::AboveThresholdPasses => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut request_data(Vec::new()),
                    GuardrailInputType::Response,
                )
                .await
                .expect("a high score passes");
        }
        Case::ResponseDetailNamesResponse => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(50.0),
            ))));
            let error = guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut request_data(Vec::new()),
                    GuardrailInputType::Response,
                )
                .await
                .expect_err("a low score blocks");
            let JudgeError::Blocked { detail } = error else {
                panic!("expected a block");
            };
            assert!(detail.error.contains("response"), "{}", detail.error);
            assert!(!detail.error.contains("request"), "{}", detail.error);
        }
    }
}
