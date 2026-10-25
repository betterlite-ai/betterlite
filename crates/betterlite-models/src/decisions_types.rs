#![cfg(test)]
use crate::decisions::types::{
    ChoiceQuestion, DecisionsRequestBody, OpenAIChoiceQuestion, OpenAIDecisionRequestBody,
    ScoreQuestion,
};

#[test]
fn records_deserialize_wire_variants() {
    let body: DecisionsRequestBody = serde_json::from_str(r#"{"state":"ticket","questions":{"urgent":{"type":"noul","instructions":"urgent?"},"kind":{"type":"choice","criteria":{"bug":null}},"score":{"type":"score","criteria":["low"]}},"future":true}"#).expect("valid body");
    assert!(body.extra.contains_key("future"));
    let _choice: ChoiceQuestion =
        serde_json::from_str(r#"{"type":"choice","criteria":{"one":null}}"#).expect("choice");
    let _score: ScoreQuestion =
        serde_json::from_str(r#"{"type":"score","criteria":["one"]}"#).expect("score");
    let _openai: OpenAIDecisionRequestBody = serde_json::from_str(
        r#"{"input":"ticket","questions":[{"type":"predicate","instructions":"urgent?"}]}"#,
    )
    .expect("openai");
}

#[test]
fn rejects_question_and_choice_bounds() {
    serde_json::from_str::<DecisionsRequestBody>(r#"{"state":null,"questions":{}}"#).unwrap_err();
    serde_json::from_str::<DecisionsRequestBody>(
        r#"{"state":null,"questions":{"q":{"type":"noul","instructions":"x"}}}"#,
    )
    .expect("minimum");
    serde_json::from_str::<ChoiceQuestion>(r#"{"type":"choice","criteria":{}}"#).unwrap_err();
    serde_json::from_str::<ScoreQuestion>(r#"{"type":"score","criteria":[]}"#).unwrap_err();
    serde_json::from_str::<OpenAIDecisionRequestBody>(r#"{"input":"x","questions":[]}"#)
        .unwrap_err();
    serde_json::from_str::<OpenAIChoiceQuestion>(
        r#"{"type":"choice","instructions":"x","choices":[{"value":"a"}]}"#,
    )
    .unwrap_err();
}
