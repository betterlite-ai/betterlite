#![cfg(test)]

use rstest::rstest;

use super::*;

#[test]
fn billed_typed_response_with_usage_is_returned_by_identity() {
    let output = format!("[{}]", message_with_text("msg_a", "hi"));
    let transformed = response_with(
        &output,
        r#","usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}"#,
    );
    let estimate = FixedEstimate::returning(9, 9);
    match billed_terminal_response(&transformed, Some(&estimate)) {
        BilledTerminal::Same(response) => assert!(std::ptr::eq(response, &transformed)),
        BilledTerminal::Estimated(_) => panic!("usage present, so the response is returned as is"),
    }
    assert_eq!(estimate.calls(), 0);
}

#[test]
fn billed_typed_response_without_usage_copies_and_estimates() {
    let transformed = response("[]");
    let estimate = FixedEstimate::returning(3, 4);
    match billed_terminal_response(&transformed, Some(&estimate)) {
        BilledTerminal::Same(_) => panic!("absent usage with an estimator makes a copy"),
        BilledTerminal::Estimated(copied) => {
            let copied_usage = copied.usage.as_ref().unwrap();
            assert_eq!(copied_usage.input_tokens, 3);
            assert_eq!(copied_usage.output_tokens, 4);
            assert_eq!(copied_usage.total_tokens, 7);
            assert!(copied_usage.input_tokens_details.is_none());
            assert!(copied_usage.cost.is_none());
        }
    }
    assert!(transformed.usage.is_none());
    assert_eq!(estimate.calls(), 1);
    let empty_estimate = FixedEstimate::empty();
    assert!(matches!(
        billed_terminal_response(&transformed, Some(&empty_estimate)),
        BilledTerminal::Estimated(_)
    ));
    let copied = match billed_terminal_response(&transformed, Some(&empty_estimate)) {
        BilledTerminal::Estimated(copied) => copied,
        BilledTerminal::Same(_) => panic!("estimator returns None, so usage stays null"),
    };
    assert!(copied.usage.is_none());
    assert_eq!(empty_estimate.calls(), 2);
}

#[test]
fn billed_typed_response_without_estimator_is_unchanged() {
    let transformed = response("[]");
    match billed_terminal_response(&transformed, None::<&FixedEstimate>) {
        BilledTerminal::Same(response) => assert!(std::ptr::eq(response, &transformed)),
        BilledTerminal::Estimated(_) => panic!("no estimator means no copy"),
    }
}

#[test]
fn billed_dict_response_preserves_existing_usage() {
    let object = json(
        r#"{"id":"r3","created_at":2,"output":[],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":0.5}}"#,
    );
    let estimate = FixedEstimate::returning(3, 4);
    let built = billed_terminal_object(&object, Some(&estimate)).unwrap();
    assert_eq!(
        built.get("usage"),
        Some(&json(
            r#"{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":0.5}"#
        ))
    );
    assert_eq!(estimate.calls(), 0);
    let empty_usage = json(r#"{"id":"r3","created_at":2,"output":[],"usage":{}}"#);
    let empty_built = billed_terminal_object(&empty_usage, Some(&estimate)).unwrap();
    assert_eq!(empty_built.get("usage"), Some(&json("{}")));
    assert_eq!(estimate.calls(), 0);
    let null_usage = json(r#"{"id":"r3","created_at":2,"output":[],"usage":null}"#);
    let null_built = billed_terminal_object(&null_usage, Some(&estimate)).unwrap();
    let inserted_usage = null_built.get("usage").expect("usage inserted");
    assert_eq!(integer(at(inserted_usage, &["total_tokens"])), 7);
    assert_eq!(estimate.calls(), 1);
}

#[test]
fn billed_dict_response_fills_defaulted_fields() {
    let object = json(r#"{"id":"r4","created_at":3,"output":[],"unknown_key":"kept"}"#);
    let estimate = FixedEstimate::returning(3, 4);
    let built = billed_terminal_object(&object, Some(&estimate)).unwrap();
    assert_eq!(estimate.calls(), 1);
    assert_eq!(built.get("id"), Some(&Json::Text("r4".to_owned())));
    assert_eq!(
        built.get("unknown_key"),
        Some(&Json::Text("kept".to_owned()))
    );
    assert_eq!(
        built.get("usage"),
        Some(&json(
            r#"{"input_tokens":3,"input_tokens_details":null,"output_tokens":4,"output_tokens_details":null,"total_tokens":7,"cost":null}"#
        ))
    );
    assert_eq!(built.len(), 24);
    let without_estimator = billed_terminal_object(&object, None::<&FixedEstimate>).unwrap();
    assert_eq!(without_estimator.get("usage"), Some(&Json::Null));
    assert_eq!(estimate.calls(), 1);
}

#[rstest]
#[case::error("error")]
#[case::incomplete_details("incomplete_details")]
#[case::instructions("instructions")]
#[case::metadata("metadata")]
#[case::model("model")]
#[case::object("object")]
#[case::parallel_tool_calls("parallel_tool_calls")]
#[case::temperature("temperature")]
#[case::tool_choice("tool_choice")]
#[case::tools("tools")]
#[case::top_p("top_p")]
#[case::max_output_tokens("max_output_tokens")]
#[case::previous_response_id("previous_response_id")]
#[case::reasoning("reasoning")]
#[case::status("status")]
#[case::text("text")]
#[case::truncation("truncation")]
#[case::user("user")]
#[case::store("store")]
fn billed_dict_response_fills_defaulted_field(#[case] name: &str) {
    let object = json(r#"{"id":"r4","created_at":3,"output":[],"unknown_key":"kept"}"#);
    let estimate = FixedEstimate::returning(3, 4);
    let built = billed_terminal_object(&object, Some(&estimate)).unwrap();
    assert_eq!(built.get(name), Some(&Json::Null));
}

#[test]
fn billed_dict_response_filters_null_reasoning_fields() {
    let object = json(
        r#"{"id":"r7","created_at":5,"output":[{"type":"reasoning","id":"rs_1","summary":[],"status":null,"content":null,"encrypted_content":null,"namespace":null}],"usage":null}"#,
    );
    let built = billed_terminal_object(&object, None::<&FixedEstimate>).unwrap();
    assert_eq!(
        built.get("output"),
        Some(&json(
            r#"[{"type":"reasoning","id":"rs_1","summary":[],"namespace":null}]"#
        ))
    );
    let second_object = json(
        r#"{"id":"r8","created_at":6,"output":[{"type":"message","id":"m1","status":null,"content":null},{"type":"reasoning","id":"rs_2","summary":[],"status":"completed","content":[]}]}"#,
    );
    let second_built = billed_terminal_object(&second_object, None::<&FixedEstimate>).unwrap();
    assert_eq!(
        second_built.get("output"),
        Some(&json(
            r#"[{"type":"message","id":"m1","status":null,"content":null},{"type":"reasoning","id":"rs_2","summary":[],"status":"completed","content":[]}]"#
        ))
    );
}

#[rstest]
#[case::null("null")]
#[case::bool("true")]
#[case::number("7")]
#[case::text(r#""text""#)]
#[case::list("[1,2]")]
fn billed_nonresponse_values_yield_none(#[case] raw: &str) {
    assert!(billed_terminal_object(&json(raw), None::<&FixedEstimate>).is_none());
}
