#![cfg(test)]

use betterlite_models::decisions::Json;

use super::pairwise_judge_response_format;

const LITELLM_FORMAT: &str = r#"{
    "type": "json_schema",
    "json_schema": {
        "schema": {
            "description": "The judge's blind A/B verdict: the response_format schema sent with the judge call\nand the validation contract on its reply. Both fields are required and preference is\nclosed over the prompt's labels, so a malformed or truncated reply is an\nunparseable-verdict error row, never a defaulted or fabricated verdict.",
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
