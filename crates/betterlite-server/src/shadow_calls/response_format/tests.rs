#![cfg(test)]

use betterlite_models::decisions::Json;

use super::pairwise_judge_response_format;

const LITELLM_FORMAT: &str = r#"{
    "type": "json_schema",
    "json_schema": {
        "schema": {
            "properties": {
                "preference": {"enum": ["A", "B", "tie"], "title": "Preference", "type": "string"},
                "confidence": {"title": "Confidence", "type": "number"}
            },
            "required": ["preference", "confidence"],
            "title": "PairwiseVerdict",
            "type": "object",
            "additionalProperties": false
        },
        "name": "PairwiseVerdict",
        "strict": true
    }
}"#;

#[test]
fn the_response_format_equals_litellms_strict_pairwise_verdict_schema() {
    let expected: Json = serde_json::from_str(LITELLM_FORMAT).unwrap();

    assert_eq!(pairwise_judge_response_format(), expected);
}
