#![cfg(test)]

use super::{Masking, apply_to_messages, apply_to_response, extract_masked_texts, update_messages};
use crate::bedrock_message_selection::wire::{Message, ModelResponse};
use crate::bedrock_records::Response;
use rstest::rstest;

#[rstest]
#[case::missing("{}", vec![])]
#[case::null(r#"{"outputs":null}"#, vec![])]
#[case::empty(r#"{"outputs":[]}"#, vec![])]
#[case::skip_absent(r#"{"outputs":[{"text":"one"},{"future":true},{"text":null},{"text":"three"}]}"#, vec!["one", "three"])]
#[case::empty_text(r#"{"outputs":[{"text":""}]}"#, vec![""])]
fn masked_outputs_skip_missing_text(#[case] wire: &str, #[case] expected: Vec<&str>) {
    let response: Response = serde_json::from_str(wire).unwrap();
    assert_eq!(extract_masked_texts(Some(&response)), expected);
    assert!(extract_masked_texts(None).is_empty());
}

#[test]
fn masking_follows_litellm_list_filtering_and_indexes_text() {
    let messages: Vec<Message> = serde_json::from_str(r#"[{"role":"user","content":[{"type":"text","text":"a","extra":7},{"type":"image"},"b",null,{"text":null}],"name":"caller"},{"role":"assistant","content":null}]"#).unwrap();
    let masked = apply_to_messages(&messages, &["A".to_owned(), "B".to_owned(), "C".to_owned()]);
    assert_eq!(
        serde_json::to_string(&masked).unwrap(),
        r#"[{"role":"user","content":[{"type":"text","text":"A","extra":7},"B",{"text":"C"}],"name":"caller"},{"role":"assistant","content":null}]"#
    );
    assert!(serde_json::to_string(&messages).unwrap().contains("image"));
}

#[test]
fn response_choice_masking_covers_message_delta_and_text() {
    let mut response: ModelResponse = serde_json::from_str(r#"{"choices":[{"message":{"content":""}},{"message":{"content":"a"}},{"delta":{"content":"b"}},{"text":"c"},{"text":"d"}],"id":"keep"}"#).unwrap();
    apply_to_response(
        &mut response,
        &["A".to_owned(), "B".to_owned(), "C".to_owned()],
    );
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        r#"{"choices":[{"message":{"content":""}},{"message":{"content":"A"}},{"delta":{"content":"B"}},{"text":"C"},{"text":"d"}],"id":"keep"}"#
    );
}

#[rstest]
#[case::enabled("{}", true, r#"[{"content":[{"type":"image"},"a"]}]"#)]
#[case::disabled("{}", false, r#"[{"content":[{"type":"image"},"a"]}]"#)]
#[case::mask_without_flag(r#"{"outputs":[{"text":"MASK"}]}"#, false, r#"[{"content":["MASK"]}]"#)]
fn updates_only_when_outputs_exist(
    #[case] wire: &str,
    #[case] enabled: bool,
    #[case] expected: &str,
) {
    let response: Response = serde_json::from_str(wire).unwrap();
    let messages: Vec<Message> =
        serde_json::from_str(r#"[{"content":[{"type":"image"},"a"]}]"#).unwrap();
    let flags = Masking {
        request: enabled,
        response: false,
    };
    assert_eq!(
        serde_json::to_string(&update_messages(&messages, &response, &flags)).unwrap(),
        expected
    );
}
