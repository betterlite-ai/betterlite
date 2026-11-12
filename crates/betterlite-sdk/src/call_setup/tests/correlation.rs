use betterlite_models::decisions::Json;

use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::correlation::ChatStream;

fn response() -> CallOutcome {
    CallOutcome::Response(Json::Text(String::from("finished")))
}

fn chat_stream() -> CallOutcome {
    CallOutcome::ChatStream(ChatStream {
        model_response: Json::Null,
        json_mode: false,
        converted_from_mock: true,
    })
}

fn responses_stream() -> CallOutcome {
    CallOutcome::ResponsesStream(Json::Text(String::from("event")))
}

#[test]
fn correlation_streaming_response_is_only_a_chat_stream() {
    assert!(chat_stream().is_streaming_response());
    assert!(!response().is_streaming_response());
    assert!(!responses_stream().is_streaming_response());
}

#[test]
fn correlation_converted_stream_is_any_stream() {
    assert!(chat_stream().is_converted_stream());
    assert!(responses_stream().is_converted_stream());
    assert!(!response().is_converted_stream());
}
