use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;

use crate::chat::messages::transform_messages;

use super::support::{body, messages};

fn transformed(source: &str) -> Vec<Message> {
    transform_messages(&messages(source)).expect("the fixture messages transform")
}

fn transformed_json(source: &str) -> Json {
    let encoded =
        serde_json::to_value(transformed(source)).expect("the transformed messages encode");
    serde_json::from_value(encoded).expect("the transformed messages read as JSON")
}

pub(super) fn strip_non_openai_message_fields() {
    assert_eq!(
        transformed_json(
            r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"t"},{"type":"text","text":"answer"}],
                "thinking_blocks":[{"type":"thinking"}],"reasoning_content":"r",
                "provider_specific_fields":{"x":1},"cache_control":{"type":"ephemeral"}}]"#
        ),
        body(r#"[{"role":"assistant","content":"answer"}]"#)
    );
    // the removal reaches nested objects and the dicts inside lists
    assert_eq!(
        transformed_json(
            r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],
                "nested":{"cache_control":{"type":"ephemeral"},"keep":1}}]"#
        ),
        body(r#"[{"role":"user","content":"ab","nested":{"keep":1}}]"#)
    );
    assert_eq!(
        transformed_json(
            r#"[{"role":"tool","content":[{"type":"text","text":"r"}],"tool_call_id":"call-1","name":"f"}]"#
        ),
        body(r#"[{"role":"tool","content":"r","name":"f","tool_call_id":"call-1"}]"#)
    );
    // the text flattening still appends search results the message carries
    assert_eq!(
        transformed_json(
            r#"[{"role":"user","content":"hi","search_results":[{"title":"T","content":[{"type":"text","text":"C"}]}]}]"#
        ),
        body(
            r#"[{"role":"user","content":"hiTC","search_results":[{"title":"T","content":[{"type":"text","text":"C"}]}]}]"#
        )
    );
}

pub(super) fn text_list_copy_behavior() {
    assert_eq!(
        transformed_json(r#"[{"role":"user","content":[]}]"#),
        body(r#"[{"role":"user","content":[]}]"#)
    );
    assert_eq!(
        transformed_json(r#"[{"role":"user","content":[{"type":"text","text":""}]}]"#),
        body(r#"[{"role":"user","content":[{"type":"text","text":""}]}]"#)
    );
    assert_eq!(
        transformed_json(r#"[{"role":"user","content":"hi"}]"#),
        body(r#"[{"role":"user","content":"hi"}]"#)
    );
    assert_eq!(
        transformed_json(
            r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}]"#
        ),
        body(r#"[{"role":"user","content":"ab"}]"#)
    );
    // an image or audio part keeps the whole list as the caller sent it
    assert_eq!(
        transformed_json(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://x/y.png"}},{"type":"text","text":"look"}]}]"#
        ),
        body(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://x/y.png"}},{"type":"text","text":"look"}]}]"#
        )
    );
    assert_eq!(
        transformed_json(
            r#"[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"AAA","format":"wav"}}]}]"#
        ),
        body(
            r#"[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"AAA","format":"wav"}}]}]"#
        )
    );
}

#[test]
fn message_transform_leaves_caller_data_untouched() {
    let original = messages(
        r#"[{"role":"assistant","content":[{"type":"text","text":"a"}],"reasoning_content":"r"}]"#,
    );
    let untouched = original.clone();
    let changed = transform_messages(&original).expect("the messages transform");
    assert_eq!(original, untouched);
    assert_ne!(changed, untouched);
}
