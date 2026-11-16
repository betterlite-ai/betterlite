#[test]
fn sse_frames_status_chunk_done_and_error_events() {
    assert_eq!(
        sse_frame(&SseEvent::Status {
            message: "Thinking...".to_owned()
        }),
        "data: {\"type\": \"status\", \"message\": \"Thinking...\"}\n\n"
    );
    assert_eq!(
        sse_frame(&SseEvent::Chunk {
            content: "hi".to_owned()
        }),
        "data: {\"type\": \"chunk\", \"content\": \"hi\"}\n\n"
    );
    assert_eq!(sse_frame(&SseEvent::Done), "data: {\"type\": \"done\"}\n\n");
    assert_eq!(
        sse_frame(&SseEvent::Error {
            message: "An internal error occurred. Please try again.".to_owned()
        }),
        ERROR_FRAME
    );
}

#[test]
fn sse_frames_a_tool_call_with_the_litellm_key_order() {
    let mut arguments = IndexMap::new();
    arguments.insert("end_date".to_owned(), "2026-01-31".to_owned());
    arguments.insert("start_date".to_owned(), "2026-01-01".to_owned());
    let event = SseEvent::ToolCall {
        tool_name: "get_usage_data".to_owned(),
        tool_label: "global usage data".to_owned(),
        arguments,
        status: ToolCallStatus::Complete,
    };
    assert_eq!(
        sse_frame(&event),
        frame_tool(
            "get_usage_data",
            "global usage data",
            &[("end_date", "2026-01-31"), ("start_date", "2026-01-01")],
            "complete"
        )
    );
    assert_eq!(
        sse_frame(&event),
        concat!(
            "data: {\"type\": \"tool_call\", \"tool_name\": \"get_usage_data\", ",
            "\"tool_label\": \"global usage data\", \"arguments\": ",
            "{\"end_date\": \"2026-01-31\", \"start_date\": \"2026-01-01\"}, ",
            "\"status\": \"complete\"}\n\n"
        )
    );
}

#[test]
fn sse_escapes_quotes_backslashes_and_control_characters() {
    let event = SseEvent::Chunk {
        content: "a\"b\\c\n\r\t\u{08}\u{0c}\u{01}".to_owned(),
    };
    assert_eq!(
        sse_frame(&event),
        "data: {\"type\": \"chunk\", \"content\": \"a\\\"b\\\\c\\n\\r\\t\\b\\f\\u0001\"}\n\n"
    );
}

#[test]
fn sse_escapes_non_ascii_like_python_json_dumps() {
    let event = SseEvent::Chunk {
        content: "é\u{7f}\u{0100}\u{ffff}😀".to_owned(),
    };
    assert_eq!(
        sse_frame(&event),
        "data: {\"type\": \"chunk\", \"content\": \"\\u00e9\\u007f\\u0100\\uffff\\ud83d\\ude00\"}\n\n"
    );
}
