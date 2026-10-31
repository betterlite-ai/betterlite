#![cfg(test)]

use rstest::rstest;

use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::unwrap_verdict_envelope;

#[rstest]
#[case::action("action")]
#[case::allowed("allowed")]
#[case::blocked("blocked")]
#[case::safe("safe")]
#[case::is_safe("is_safe")]
#[case::decision("decision")]
#[case::verdict("verdict")]
#[case::status("status")]
#[case::score("score")]
#[case::risk_score("risk_score")]
#[case::confidence("confidence")]
#[case::categories("categories")]
#[case::classifications("classifications")]
#[case::violations("violations")]
#[case::threats("threats")]
#[case::policies("policies")]
#[case::reason("reason")]
#[case::rules("rules")]
#[case::sanitized_text("sanitized_text")]
#[case::sanitized_text_camel("sanitizedText")]
#[case::sanitized_payload("sanitized_payload")]
fn cisco_verdict_null_is_present(#[case] key: &str) {
    let response = Json::parse(&format!(
        r#"{{"{key}":null,"result":{{"action":"allow"}}}}"#
    ));
    assert!(super::has_decision_fields(&response));
    let unwrapped = unwrap_verdict_envelope(&response);
    let Json::Object(fields) = unwrapped else {
        panic!("the verdict must remain an object");
    };
    assert!(fields.contains_key("result"));
    assert_eq!(fields.get(key).unwrap().get(), "null");
}

#[rstest]
#[case::empty("{}")]
#[case::unknown(r#"{"unknown":null}"#)]
#[case::nested(r#"{"result":{"action":"allow"}}"#)]
#[case::scalar("null")]
fn cisco_verdict_absent_decision(#[case] input: &str) {
    assert!(!super::has_decision_fields(&Json::parse(input)));
}
