use crate::transcription::reply::decode_transcription_reply;

const NONTEXT: &str = "Bedrock response contains non-text transcript content";

const REPLY_ROWS: [(&str, Result<&str, &str>); 22] = [
    (
        r#"{"output":{"message":{"content":[{"text":"hello "},{"text":"world"}]}}}"#,
        Ok("hello world"),
    ),
    (r#"{"output":{"message":{"content":[]}}}"#, Ok("")),
    (
        r#"{"output":{"message":{"content":[{"text":"a","extra":[1]}]}},"stopReason":null}"#,
        Ok("a"),
    ),
    (
        r#"{"output":{"message":{"content":[{"text":"a"}]}},"stopReason":"end_turn","usage":{"inputTokens":1,"outputTokens":2,"totalTokens":3,"cacheReadInputTokens":4,"cacheWriteInputTokens":5}}"#,
        Ok("a"),
    ),
    (
        r#"{"output":{"message":{"content":[{"text":"a"}]}},"usage":{"inputTokens":0,"outputTokens":0}}"#,
        Ok("a"),
    ),
    (
        r#"{"output":{"message":{"content":[{"toolUse":{}}]}}}"#,
        Err(NONTEXT),
    ),
    (r#"{"output":{"message":{"content":[1]}}}"#, Err(NONTEXT)),
    (r#"{"output":{"message":{"content":[null]}}}"#, Err(NONTEXT)),
    (
        r#"{"output":{"message":{"content":[{"text":"a"},{"image":1}]}}}"#,
        Err(NONTEXT),
    ),
    (
        r#"{"output":{"message":{"content":[{"text":1}]}}}"#,
        Err("invalid Converse response: invalid type for `text`, expected a string"),
    ),
    (
        r#"{"output":{"message":{"content":[{"toolUse":1},{"text":3}]}}}"#,
        Err("invalid Converse response: invalid type for `text`"),
    ),
    (
        r#"{"output":{"message":{"content":[]}},"stopReason":5}"#,
        Err("invalid Converse response: invalid type: integer `5`"),
    ),
    (
        r#"{"output":{"message":{"content":[]}},"usage":null}"#,
        Err("invalid Converse response: invalid type: null"),
    ),
    (
        r#"{"output":{"message":{"content":[]}},"usage":{"outputTokens":1}}"#,
        Err("invalid Converse response: missing field `inputTokens`"),
    ),
    (
        r#"{"output":{"message":{"content":[]}},"usage":{"inputTokens":1}}"#,
        Err("invalid Converse response: missing field `outputTokens`"),
    ),
    (
        r#"{"output":{"message":{"content":[]}},"usage":{"inputTokens":-1,"outputTokens":1}}"#,
        Err("invalid Converse response: invalid value: integer `-1`"),
    ),
    (
        r#"{"output":{"message":{"content":[]}},"usage":{"inputTokens":1,"outputTokens":1,"totalTokens":"x"}}"#,
        Err("invalid Converse response: invalid type: string"),
    ),
    (
        r#"{"output":{"message":{"content":[]}},"usage":{"inputTokens":1,"outputTokens":1,"cacheReadInputTokens":null}}"#,
        Err("invalid Converse response: invalid type: null"),
    ),
    (
        r#"{"output":{"message":{}}}"#,
        Err("invalid Converse response: missing field `content`"),
    ),
    (
        r#"{"output":{}}"#,
        Err("invalid Converse response: missing field `message`"),
    ),
    (
        "{}",
        Err("invalid Converse response: missing field `output`"),
    ),
    ("[", Err("invalid Converse response: EOF while parsing")),
];

pub(super) fn native_transcript_reply() {
    for (source, expected) in REPLY_ROWS {
        let decoded = decode_transcription_reply(source.as_bytes());
        match expected {
            Ok(text) => assert_eq!(decoded.unwrap().text, text, "{source}"),
            Err(prefix) => {
                let message = decoded.unwrap_err().to_string();
                assert!(message.starts_with(prefix), "{source}: {message}");
            }
        }
    }
}
