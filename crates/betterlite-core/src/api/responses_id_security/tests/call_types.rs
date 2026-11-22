use rstest::rstest;

use crate::api::responses_id_security::ResponsesCall;

#[rstest]
#[case::aresponses("aresponses", ResponsesCall::Create)]
#[case::get_responses("aget_responses", ResponsesCall::Get)]
#[case::delete_responses("adelete_responses", ResponsesCall::Delete)]
#[case::cancel_responses("acancel_responses", ResponsesCall::Cancel)]
#[case::list_input_items("alist_input_items", ResponsesCall::ListInputItems)]
#[case::other_call_type("acompletion", None)]
fn call_type_mapping(#[case] call_type: &str, #[case] expected: Option<ResponsesCall>) {
    assert_eq!(
        ResponsesCall::from_call_type(call_type),
        expected,
        "{call_type}"
    );
}
