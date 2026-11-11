use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::apply::{OnFailure, RequestData};
use crate::judge::verdict::{EvalInformation, EvaluationInformation};

use super::support::{
    Outcome, RecordingCompletion, guardrail_with, reply_with_score, request_data, texts,
};

fn information(score: f64) -> EvaluationInformation {
    EvaluationInformation {
        eval_name: "earlier".to_owned(),
        overall_score: score,
        passed: true,
        judge_model: "gpt-4o-mini".to_owned(),
        threshold: 80.0,
        verdicts: Json::List(Vec::new()),
    }
}

#[derive(Debug, Clone, Copy)]
enum Case {
    MissingOverallScore,
    ClampHigh,
    ClampLow,
    TextScore,
    NullScore,
    BooleanScore,
    PassedAtThreshold,
    PassedJustBelow,
    EvaluationFields,
    SecondJudgeMerges,
    ExistingSingleBecomesList,
    ExistingListGrows,
}

#[rstest]
#[case::missing_overall_score(Case::MissingOverallScore)]
#[case::clamp_high(Case::ClampHigh)]
#[case::clamp_low(Case::ClampLow)]
#[case::text_score(Case::TextScore)]
#[case::null_score(Case::NullScore)]
#[case::boolean_score(Case::BooleanScore)]
#[case::passed_at_threshold(Case::PassedAtThreshold)]
#[case::passed_just_below(Case::PassedJustBelow)]
#[case::evaluation_fields(Case::EvaluationFields)]
#[case::second_judge_merges(Case::SecondJudgeMerges)]
#[case::existing_single_becomes_list(Case::ExistingSingleBecomesList)]
#[case::existing_list_grows(Case::ExistingListGrows)]
#[tokio::test]
async fn judge_verdict_tests_behavior(#[case] case: Case) {
    match case {
        Case::MissingOverallScore => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                r#"{"verdicts": []}"#.to_owned(),
            ))));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("a missing score defaults to 100");
            let information = single(&data);
            assert_eq!(information.overall_score.to_bits(), 100.0_f64.to_bits());
            assert!(information.passed);
            assert_eq!(information.verdicts, Json::List(Vec::new()));
        }
        Case::ClampHigh => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(150.0),
            ))));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("a high score clamps to 100");
            assert_eq!(single(&data).overall_score.to_bits(), 100.0_f64.to_bits());
        }
        Case::ClampLow => {
            let mut guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(-10.0),
            ))));
            guardrail.on_failure = OnFailure::Log;
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("a low score clamps to 0 and only logs");
            let information = single(&data);
            assert_eq!(information.overall_score.to_bits(), 0.0_f64.to_bits());
            assert!(!information.passed);
        }
        Case::TextScore => {
            let mut data = request_data(Vec::new());
            let returned = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                r#"{"overall_score": "90"}"#.to_owned(),
            ))))
            .apply_guardrail(
                texts(&["response"]),
                &mut data,
                GuardrailInputType::Response,
            )
            .await
            .expect("a text score fails open");
            assert_eq!(returned.texts, Some(vec!["response".to_owned()]));
            assert_eq!(data.metadata.eval_information, None);
        }
        Case::NullScore => {
            let mut data = request_data(Vec::new());
            guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                r#"{"overall_score": null}"#.to_owned(),
            ))))
            .apply_guardrail(
                texts(&["response"]),
                &mut data,
                GuardrailInputType::Response,
            )
            .await
            .expect("a null score fails open");
            assert_eq!(data.metadata.eval_information, None);
        }
        Case::BooleanScore => {
            let mut data = request_data(Vec::new());
            guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                r#"{"overall_score": true}"#.to_owned(),
            ))))
            .apply_guardrail(
                texts(&["response"]),
                &mut data,
                GuardrailInputType::Response,
            )
            .await
            .expect("a boolean score fails open");
            assert_eq!(data.metadata.eval_information, None);
        }
        Case::PassedAtThreshold => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(80.0),
            ))));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("a score equal to the threshold passes");
            assert!(single(&data).passed);
        }
        Case::PassedJustBelow => {
            let mut guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(79.9),
            ))));
            guardrail.on_failure = OnFailure::Log;
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("log mode does not block");
            assert!(!single(&data).passed);
        }
        Case::EvaluationFields => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("judge passes");
            let information = single(&data);
            assert_eq!(information.eval_name, "test_judge");
            assert_eq!(information.judge_model, "gpt-4o-mini");
            assert_eq!(information.threshold.to_bits(), 80.0_f64.to_bits());
            assert_eq!(information.overall_score.to_bits(), 90.0_f64.to_bits());
            let Json::List(verdicts) = &information.verdicts else {
                panic!("the reply carries a verdict list");
            };
            assert_eq!(verdicts.len(), 1);
        }
        Case::SecondJudgeMerges => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(texts(&["first"]), &mut data, GuardrailInputType::Response)
                .await
                .expect("first judge passes");
            guardrail
                .apply_guardrail(texts(&["second"]), &mut data, GuardrailInputType::Response)
                .await
                .expect("second judge passes");
            let EvalInformation::Many(entries) = data.metadata.eval_information.expect("entries")
            else {
                panic!("two judges merge into a list");
            };
            assert_eq!(entries.len(), 2);
        }
        Case::ExistingSingleBecomesList => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            let mut data = request_data(Vec::new());
            data.metadata.eval_information = Some(EvalInformation::Single(information(10.0)));
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("judge passes");
            let EvalInformation::Many(entries) = data.metadata.eval_information.expect("entries")
            else {
                panic!("an earlier single entry becomes the first of a list");
            };
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].overall_score.to_bits(), 10.0_f64.to_bits());
            assert_eq!(entries[1].overall_score.to_bits(), 90.0_f64.to_bits());
        }
        Case::ExistingListGrows => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                reply_with_score(90.0),
            ))));
            let mut data = request_data(Vec::new());
            data.metadata.eval_information = Some(EvalInformation::Many(vec![information(10.0)]));
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("judge passes");
            let EvalInformation::Many(entries) = data.metadata.eval_information.expect("entries")
            else {
                panic!("an earlier list keeps growing");
            };
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].overall_score.to_bits(), 10.0_f64.to_bits());
        }
    }
}

fn single(data: &RequestData) -> EvaluationInformation {
    let EvalInformation::Single(information) = data
        .metadata
        .eval_information
        .clone()
        .expect("eval information")
    else {
        panic!("one judge leaves one entry");
    };
    information
}
