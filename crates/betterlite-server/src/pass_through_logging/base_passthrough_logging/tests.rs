#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::request_headers::Settings;
use betterlite_logging::passthrough_handler::types::{NO_COMPLETE_RESPONSE, StreamedResponse};
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::Level;

use super::{
    CollectedChunksInput, PassthroughChatInput, PassthroughLoggingError, PassthroughPayload as _,
    PassthroughPayloadInput, ResponsePayloadBuilder, UpstreamChatReply,
};
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;
use support::{
    END, FakePayload, FakeProvider, FixedCost, RecordingBuilder, START, capture_logs, field,
    json_object, logger, logging_state, object, timestamp,
};

pub(crate) mod support;

#[test]
fn chat_uses_the_request_model_for_the_transform_and_the_payload() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let response = object(r#"{"model": "served-model"}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload)
        .passthrough_chat_handler(PassthroughChatInput {
            upstream: UpstreamChatReply {
                status_code: 200,
                body: b"{}",
            },
            response_body: &response,
            call_log: &mut state,
            url_route: "/v2/chat",
            start_time: timestamp(START),
            end_time: timestamp(END),
            request_body: &request,
            kwargs: ResponseLoggingKwargs::default(),
        })
        .expect("the chat handler succeeds");
    assert_eq!(provider.seen(), ["command-r"]);
    assert_eq!(payload.seen(), ["command-r"]);
    let Some(PassThroughResponse::ModelResponse(result)) = outcome.result else {
        panic!("the result is a model response");
    };
    assert_eq!(
        field(&result, "payload_model"),
        &Json::Text("command-r".to_owned())
    );
    assert_eq!(outcome.kwargs.model, Some("command-r".to_owned()));
}

#[test]
fn chat_takes_the_model_from_the_response_body_when_the_request_omits_it() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object("{}");
    let response = object(r#"{"model": "served-model"}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload)
        .passthrough_chat_handler(PassthroughChatInput {
            upstream: UpstreamChatReply {
                status_code: 200,
                body: b"{}",
            },
            response_body: &response,
            call_log: &mut state,
            url_route: "/v2/chat",
            start_time: timestamp(START),
            end_time: timestamp(END),
            request_body: &request,
            kwargs: ResponseLoggingKwargs::default(),
        })
        .expect("the chat handler succeeds");
    assert_eq!(provider.seen(), ["served-model"]);
    assert_eq!(outcome.kwargs.model, Some("served-model".to_owned()));
}

#[test]
fn chat_takes_a_null_request_model_as_absent() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": null}"#);
    let response = object(r#"{"model": "served-model"}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload)
        .passthrough_chat_handler(PassthroughChatInput {
            upstream: UpstreamChatReply {
                status_code: 200,
                body: b"{}",
            },
            response_body: &response,
            call_log: &mut state,
            url_route: "/v2/chat",
            start_time: timestamp(START),
            end_time: timestamp(END),
            request_body: &request,
            kwargs: ResponseLoggingKwargs::default(),
        })
        .expect("the chat handler succeeds");
    assert_eq!(provider.seen(), ["served-model"]);
    assert_eq!(outcome.kwargs.model, Some("served-model".to_owned()));
}

#[test]
fn chat_uses_the_empty_model_when_neither_body_names_one() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object("{}");
    let response = object(r#"{"model": null}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload)
        .passthrough_chat_handler(PassthroughChatInput {
            upstream: UpstreamChatReply {
                status_code: 200,
                body: b"{}",
            },
            response_body: &response,
            call_log: &mut state,
            url_route: "/v2/chat",
            start_time: timestamp(START),
            end_time: timestamp(END),
            request_body: &request,
            kwargs: ResponseLoggingKwargs::default(),
        })
        .expect("the chat handler succeeds");
    assert_eq!(provider.seen(), [""]);
    assert_eq!(outcome.kwargs.model, Some(String::new()));
}

#[test]
fn chat_returns_the_model_type_error_for_a_non_text_request_model() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": true}"#);
    let response = object(r#"{"model": "served-model"}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload).passthrough_chat_handler(PassthroughChatInput {
        upstream: UpstreamChatReply {
            status_code: 200,
            body: b"{}",
        },
        response_body: &response,
        call_log: &mut state,
        url_route: "/v2/chat",
        start_time: timestamp(START),
        end_time: timestamp(END),
        request_body: &request,
        kwargs: ResponseLoggingKwargs::default(),
    });
    let Err(error) = outcome else {
        panic!("the handler rejects a boolean model");
    };
    assert!(matches!(
        error,
        PassthroughLoggingError::ModelType {
            field: "request_body.model"
        }
    ));
    assert!(provider.seen().is_empty());
    assert!(payload.seen().is_empty());
}

#[test]
fn chat_returns_the_model_type_error_for_a_non_text_response_model() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object("{}");
    let response = object(r#"{"model": 3}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload).passthrough_chat_handler(PassthroughChatInput {
        upstream: UpstreamChatReply {
            status_code: 200,
            body: b"{}",
        },
        response_body: &response,
        call_log: &mut state,
        url_route: "/v2/chat",
        start_time: timestamp(START),
        end_time: timestamp(END),
        request_body: &request,
        kwargs: ResponseLoggingKwargs::default(),
    });
    let Err(error) = outcome else {
        panic!("the handler rejects a numeric response model");
    };
    assert!(matches!(
        error,
        PassthroughLoggingError::ModelType {
            field: "response_body.model"
        }
    ));
}

#[test]
fn chat_returns_the_transform_error_before_the_payload_runs() {
    let provider = Arc::new(FakeProvider::failing());
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = logger(&provider, &payload).passthrough_chat_handler(PassthroughChatInput {
        upstream: UpstreamChatReply {
            status_code: 200,
            body: b"{}",
        },
        response_body: &response,
        call_log: &mut state,
        url_route: "/v2/chat",
        start_time: timestamp(START),
        end_time: timestamp(END),
        request_body: &request,
        kwargs: ResponseLoggingKwargs::default(),
    });
    let Err(error) = outcome else {
        panic!("a failing transform fails the handler");
    };
    assert!(matches!(error, PassthroughLoggingError::Transform { .. }));
    assert!(payload.seen().is_empty());
}

#[test]
fn chat_returns_the_kwargs_the_payload_built() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let built = ResponseLoggingKwargs {
        response_cost: Some(Decimal::new(250, 2)),
        model: None,
        litellm_params: object(r#"{"api_base": "https://api.cohere.ai"}"#),
        standard_logging_object: Some(Json::Text("object".to_owned())),
        passthrough_request_body: Some(object(r#"{"user": "alice"}"#)),
    };
    let payload = Arc::new(FakePayload::new(built.clone()));
    let request = object(r#"{"model": "command-r"}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = logger(&provider, &payload)
        .passthrough_chat_handler(PassthroughChatInput {
            upstream: UpstreamChatReply {
                status_code: 200,
                body: b"{}",
            },
            response_body: &response,
            call_log: &mut state,
            url_route: "/v2/chat",
            start_time: timestamp(START),
            end_time: timestamp(END),
            request_body: &request,
            kwargs: ResponseLoggingKwargs::default(),
        })
        .expect("the chat handler succeeds");
    let mut expected = built;
    expected.model = Some("command-r".to_owned());
    assert_eq!(outcome.kwargs, expected);
}

#[test]
fn chat_hands_the_call_log_to_the_payload() {
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let response = object("{}");
    let mut state = logging_state();
    let _outcome = logger(&provider, &payload)
        .passthrough_chat_handler(PassthroughChatInput {
            upstream: UpstreamChatReply {
                status_code: 200,
                body: b"{}",
            },
            response_body: &response,
            call_log: &mut state,
            url_route: "/v2/chat",
            start_time: timestamp(START),
            end_time: timestamp(END),
            request_body: &request,
            kwargs: ResponseLoggingKwargs::default(),
        })
        .expect("the chat handler succeeds");
    assert_eq!(payload.call_ids(), ["call-1"]);
}

#[test]
fn chat_payload_builder_runs_the_response_payload_with_the_end_user() {
    let cost = Arc::new(FixedCost::new(Decimal::new(5, 1)));
    let builder = Arc::new(RecordingBuilder::default());
    let production = ResponsePayloadBuilder::new(
        Arc::<FixedCost>::clone(&cost),
        Arc::<RecordingBuilder>::clone(&builder),
        Settings::default(),
        false,
    );
    let mut response = json_object(r#"{"choices": []}"#);
    let mut state = logging_state();
    let kwargs = production.build(PassthroughPayloadInput {
        response: &mut response,
        model: "command-r",
        kwargs: ResponseLoggingKwargs {
            response_cost: None,
            model: None,
            litellm_params: IndexMap::new(),
            standard_logging_object: None,
            passthrough_request_body: Some(object(r#"{"user": "alice"}"#)),
        },
        start_time: timestamp(START),
        end_time: timestamp(END),
        call_log: &mut state,
    });
    assert_eq!(cost.models(), ["command-r"]);
    assert_eq!(builder.responses(), [json_object(r#"{"choices": []}"#)]);
    assert_eq!(kwargs.response_cost, Some(Decimal::new(5, 1)));
    assert_eq!(kwargs.model.as_deref(), Some("command-r"));
    assert_eq!(
        kwargs.litellm_params.get("proxy_server_request"),
        Some(&json_object(r#"{"body": {"user": "alice"}}"#))
    );
}

#[test]
fn chat_payload_builder_reads_the_end_user_from_the_request_body() {
    let production = ResponsePayloadBuilder::new(
        Arc::new(FixedCost::new(Decimal::ZERO)),
        Arc::new(RecordingBuilder::default()),
        Settings::default(),
        false,
    );
    assert_eq!(
        production.end_user(Some(&object(r#"{"user": "bob"}"#))),
        Some("bob".to_owned())
    );
    assert_eq!(production.end_user(Some(&object(r#"{"user": ""}"#))), None);
    assert_eq!(production.end_user(None), None);
}

#[test]
fn chunks_returns_the_built_response_with_the_payload_kwargs() {
    let streamed = StreamedResponse::Model(json_object(r#"{"id": "chatcmpl-1"}"#));
    let provider = Arc::new(FakeProvider::streaming(Some(streamed)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload)
        .log_collected_chunks(CollectedChunksInput {
            call_log: &mut state,
            request_body: &request,
            start_time: timestamp(START),
            end_time: timestamp(END),
            all_chunks: &["{}".to_owned()],
        })
        .expect("the collected chunks are built");
    assert_eq!(provider.seen(), ["command-r"]);
    assert_eq!(payload.seen(), ["command-r"]);
    let Some(PassThroughResponse::ModelResponse(result)) = outcome.result else {
        panic!("the result is a model response");
    };
    assert_eq!(
        field(&result, "payload_model"),
        &Json::Text("command-r".to_owned())
    );
    assert_eq!(outcome.kwargs.model, Some("command-r".to_owned()));
}

#[test]
fn chunks_returns_a_text_completion_response_as_given() {
    let streamed = StreamedResponse::TextCompletion(json_object(r#"{"id": "cmpl-1"}"#));
    let provider = Arc::new(FakeProvider::streaming(Some(streamed)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload)
        .log_collected_chunks(CollectedChunksInput {
            call_log: &mut state,
            request_body: &request,
            start_time: timestamp(START),
            end_time: timestamp(END),
            all_chunks: &["{}".to_owned()],
        })
        .expect("the collected chunks are built");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::TextCompletion(_))
    ));
}

#[test]
fn chunks_logs_the_error_text_and_returns_no_result_for_a_none_build() {
    let provider = Arc::new(FakeProvider::streaming(None));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let mut state = logging_state();
    let (buffer, _guard) = capture_logs(Level::ERROR);
    let outcome = logger(&provider, &payload)
        .log_collected_chunks(CollectedChunksInput {
            call_log: &mut state,
            request_body: &request,
            start_time: timestamp(START),
            end_time: timestamp(END),
            all_chunks: &[],
        })
        .expect("a missing build is not an error");
    assert!(buffer.contents().contains(NO_COMPLETE_RESPONSE));
    assert!(outcome.result.is_none());
    assert_eq!(outcome.kwargs, ResponseLoggingKwargs::default());
    assert!(payload.seen().is_empty());
}

#[test]
fn chunks_returns_the_stream_build_error() {
    let provider = Arc::new(FakeProvider::failing());
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload).log_collected_chunks(CollectedChunksInput {
        call_log: &mut state,
        request_body: &request,
        start_time: timestamp(START),
        end_time: timestamp(END),
        all_chunks: &["{}".to_owned()],
    });
    let Err(error) = outcome else {
        panic!("a failing stream build fails the handler");
    };
    assert!(matches!(error, PassthroughLoggingError::StreamBuild { .. }));
    assert!(payload.seen().is_empty());
}

#[test]
fn chunks_returns_the_model_type_error_without_calling_the_provider() {
    let provider = Arc::new(FakeProvider::streaming(None));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": []}"#);
    let mut state = logging_state();
    let outcome = logger(&provider, &payload).log_collected_chunks(CollectedChunksInput {
        call_log: &mut state,
        request_body: &request,
        start_time: timestamp(START),
        end_time: timestamp(END),
        all_chunks: &["{}".to_owned()],
    });
    let Err(error) = outcome else {
        panic!("the handler rejects a list model");
    };
    assert!(matches!(
        error,
        PassthroughLoggingError::ModelType {
            field: "request_body.model"
        }
    ));
    assert!(provider.seen().is_empty());
}
