use rstest::rstest;
use serde_json::{Value, json};

use super::support::{built, mapping, options};
use crate::spend_event::input::{CachePresence, ResponseBody, ResponseObject};

const FIELDS: &str = r#"{
    "id": "resp-1", "model": "gpt-4", "usage": {"prompt_tokens": 3}, "usage_info": null,
    "status": "completed", "background": false, "object": "response",
    "choices": [{"index": 0, "message": {"content": "hello"}}], "output": [{"type": "message"}]
}"#;

fn body(fields: &str, names: &[&str]) -> ResponseBody {
    ResponseBody {
        fields: mapping(fields),
        tool_call_names: names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn compact(response: ResponseObject) -> Value {
    let mut call = options(true, CachePresence::On);
    call.response = response;
    built("{}", &call)["response"].clone()
}

#[rstest]
#[case::model(ResponseObject::Model(body(FIELDS, &[])))]
#[case::mapping(ResponseObject::Mapping(body(FIELDS, &[])))]
fn keeps_only_the_identity_and_usage_scalars(#[case] response: ResponseObject) {
    let expected = json!({
        "id": "resp-1", "model": "gpt-4", "usage": {"prompt_tokens": 3_u8}, "usage_info": null,
        "status": "completed", "background": false
    });
    assert_eq!(compact(response), expected);
}

#[test]
fn a_response_that_lacks_a_key_does_not_gain_it() {
    let compacted = compact(ResponseObject::Mapping(body(r#"{"id": "only"}"#, &[])));
    assert_eq!(compacted, json!({"id": "only"}));
}

#[test]
fn called_tools_become_one_synthetic_assistant_choice_in_call_order() {
    let compacted = compact(ResponseObject::Model(body(
        r#"{"id": "r"}"#,
        &["b", "a", "b"],
    )));
    let call = |name: &str| json!({"id": null, "type": "function", "function": {"name": name, "arguments": "{}"}});
    let expected = json!({
        "id": "r",
        "choices": [{"index": 0_u8, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": null,
            "tool_calls": [call("b"), call("a"), call("b")]}}]
    });
    assert_eq!(compacted, expected);
}

#[test]
fn an_absent_response_is_null() {
    assert_eq!(compact(ResponseObject::Absent), Value::Null);
}

#[rstest]
#[case::absent(ResponseObject::Absent, true)]
#[case::model(ResponseObject::Model(ResponseBody::default()), true)]
#[case::mapping(ResponseObject::Mapping(ResponseBody::default()), true)]
#[case::batch(ResponseObject::Batch(ResponseBody::default()), false)]
#[case::interactions(ResponseObject::Interactions(ResponseBody::default()), false)]
fn keeps_batch_and_interaction_responses_in_process(
    #[case] response: ResponseObject,
    #[case] offloadable: bool,
) {
    assert_eq!(response.is_offloadable(), offloadable);
}
