use crate::api::responses_id_security::ResponsesCall;

#[test]
fn call_type_mapping() {
    let cases = [
        ("aresponses", ResponsesCall::Create),
        ("aget_responses", ResponsesCall::Get),
        ("adelete_responses", ResponsesCall::Delete),
        ("acancel_responses", ResponsesCall::Cancel),
        ("alist_input_items", ResponsesCall::ListInputItems),
    ];
    for (call_type, expected) in cases {
        assert_eq!(
            ResponsesCall::from_call_type(call_type),
            Some(expected),
            "{call_type}"
        );
    }
    assert_eq!(ResponsesCall::from_call_type("acompletion"), None);
}
