#![cfg(test)]
use crate::decisions::types::{NoulQuestion, OpenAIChoiceQuestion};

#[test]
fn validators_reject_invalid_questions() {
    serde_json::from_str::<NoulQuestion>(r#"{"type":"noul"}"#).unwrap_err();
    serde_json::from_str::<OpenAIChoiceQuestion>(
        r#"{"type":"choice","instructions":"pick","choices":[{"value":true},{"value":"true"}]}"#,
    )
    .unwrap_err();
}
