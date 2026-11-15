use super::support::{PROMPT, id, new_input, send, try_send};
use crate::realtime_input::error::InputError;

const CREATE: &str = r#"{"type":"conversation.item.create","item":"#;

fn text_events(content: &str, text: &str) -> [String; 3] {
    [
        format!(
            r#"{{"event":{{"contentStart":{{"promptName":"{PROMPT}","contentName":"{content}","type":"TEXT","interactive":true,"role":"USER","textInputConfiguration":{{"mediaType":"text/plain"}}}}}}}}"#
        ),
        format!(
            r#"{{"event":{{"textInput":{{"promptName":"{PROMPT}","contentName":"{content}","content":{text}}}}}}}"#
        ),
        format!(
            r#"{{"event":{{"contentEnd":{{"promptName":"{PROMPT}","contentName":"{content}"}}}}}}"#
        ),
    ]
}

fn tool_events(content: &str, call_id: &str, output: &str) -> [String; 3] {
    [
        format!(
            r#"{{"event":{{"contentStart":{{"promptName":"{PROMPT}","contentName":"{content}","type":"TOOL","interactive":false,"role":"TOOL","toolResultInputConfiguration":{{"toolUseId":"{call_id}","type":"TEXT","textInputConfiguration":{{"mediaType":"text/plain"}}}}}}}}}}"#
        ),
        format!(
            r#"{{"event":{{"toolResult":{{"promptName":"{PROMPT}","contentName":"{content}","content":{output}}}}}}}"#
        ),
        format!(
            r#"{{"event":{{"contentEnd":{{"promptName":"{PROMPT}","contentName":"{content}"}}}}}}"#
        ),
    ]
}

pub(super) fn conversation_text_item() {
    let (mut input, ids) = new_input();
    assert!(!input.session.prompt_started);
    let message = format!(
        r#"{CREATE}{{"type":"message","role":"assistant","content":[{{"type":"input_text","text":"hi"}},{{"type":"input_audio","audio":"x"}},{{"type":"input_text"}},{{"type":"input_text","text":"bye"}}]}}}}"#
    );
    let mut expected = Vec::new();
    expected.extend(text_events(&id(4), r#""hi""#));
    expected.extend(text_events(&id(5), r#""""#));
    expected.extend(text_events(&id(6), r#""bye""#));
    assert_eq!(send(&mut input, &ids, &message), expected);
    assert!(!input.session.client_audio_streamed);
    assert_eq!(input.open_audio, None);
    for ignored in [
        r#"{"type":"message"}"#,
        r#"{"type":"message","content":[]}"#,
        r#"{"type":"reference","content":[{"type":"input_text","text":"x"}]}"#,
        r#"{"content":[{"type":"input_text","text":"x"}]}"#,
        r#"{"type":5}"#,
        "{}",
    ] {
        assert!(send(&mut input, &ids, &format!("{CREATE}{ignored}}}")).is_empty());
    }
    assert!(send(&mut input, &ids, r#"{"type":"conversation.item.create"}"#).is_empty());
    for malformed in [
        "null",
        "5",
        r#"{"type":"message","content":"hi"}"#,
        r#"{"type":"message","content":[{"type":"input_text","text":5}]}"#,
    ] {
        let result = try_send(&mut input, &ids, &format!("{CREATE}{malformed}}}"));
        assert!(matches!(result, Err(InputError::Malformed { .. })));
    }
}

pub(super) fn conversation_tool_result() {
    let (mut input, ids) = new_input();
    let call = |output: &str| {
        format!(
            r#"{CREATE}{{"type":"function_call_output","id":"item_supplied","call_id":"call_1","output":{output}}}}}"#
        )
    };
    assert_eq!(
        send(&mut input, &ids, &call(r#""sunny""#)),
        tool_events(&id(4), "call_1", r#""sunny""#)
    );
    assert_eq!(
        send(&mut input, &ids, &call(r#"{"temp":21,"unit":"c"}"#)),
        tool_events(&id(5), "call_1", r#""{\"temp\": 21, \"unit\": \"c\"}""#)
    );
    assert_eq!(
        send(&mut input, &ids, &call("[1, 2]")),
        tool_events(&id(6), "call_1", r#""[1, 2]""#)
    );
    assert_eq!(
        send(&mut input, &ids, &call("null")),
        tool_events(&id(7), "call_1", r#""null""#)
    );
    let bare = format!(r#"{CREATE}{{"type":"function_call_output"}}}}"#);
    assert_eq!(
        send(&mut input, &ids, &bare),
        tool_events(&id(8), "", r#""""#)
    );
}
