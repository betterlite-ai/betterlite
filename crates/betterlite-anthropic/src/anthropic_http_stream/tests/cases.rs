#![cfg(test)]

use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;

use crate::anthropic_http_stream::call::{StreamCall, StreamClients, StreamOpening};
use crate::anthropic_http_stream::completion::{
    CompletionStream, WrappedStream, acompletion_stream_function,
};
use crate::anthropic_http_stream::error::{PostFailure, StreamOpenError};
use crate::tool_names::REVERSE_MAP_KEY;

use super::fixture::{
    API_BASE, DATA, ERROR_TEXT, FRAME, FixedCounter, FixedIds, FixtureFailure, FixtureTransport,
    RecordingLogger, SECRET, TIMEOUT, assert_no_post_call, assert_post_call, exception_headers,
    header, messages, request_headers, response_headers, reverse_map, stream_build,
    stream_build_with, stream_call,
};

#[derive(Clone, Copy, Debug)]
enum Scenario {
    StreamPost,
    HandshakeError,
    NonHttpError500,
    SyncStatusGuard,
    StreamFlagAndContext,
    ResponseHeaders,
}

#[rstest]
#[case::stream_post(Scenario::StreamPost)]
#[case::handshake_error(Scenario::HandshakeError)]
#[case::non_http_error_500(Scenario::NonHttpError500)]
#[case::sync_status_guard(Scenario::SyncStatusGuard)]
#[case::stream_flag_and_context(Scenario::StreamFlagAndContext)]
#[case::response_headers(Scenario::ResponseHeaders)]
#[tokio::test]
async fn anthropic_http_stream_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::StreamPost => stream_post().await,
        Scenario::HandshakeError => handshake_error().await,
        Scenario::NonHttpError500 => non_http_error_500().await,
        Scenario::SyncStatusGuard => sync_status_guard().await,
        Scenario::StreamFlagAndContext => stream_flag_and_context().await,
        Scenario::ResponseHeaders => response_headers_case().await,
    }
}

async fn stream_post() {
    async_stream_post(true).await;
    async_stream_post(false).await;
    sync_stream_post(true).await;
    sync_stream_post(false).await;
}

async fn async_stream_post(supplied: bool) {
    let messages = messages();
    let reverse = reverse_map();
    let supplied_client = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let module_async = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let module_blocking = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: supplied.then_some(&supplied_client),
        module_level_aclient: &module_async,
        module_level_client: &module_blocking,
        logger: &logger,
    };
    let call = stream_call(DATA, &messages, &reverse);
    let opened = StreamOpening {
        clients,
        call: &call,
        build: stream_build(&FixedIds),
    }
    .make_call()
    .await
    .expect("a 200 reply opens the stream");
    let (served, idle) = if supplied {
        (&supplied_client, [&module_async, &module_blocking])
    } else {
        (&module_async, [&supplied_client, &module_blocking])
    };
    assert_served(served, &idle);
    assert!(!opened.sync_stream);
    assert_eq!(header(&opened.headers, "x-request-id"), Some("req-1"));
    let stream = &opened.completion_stream;
    assert_eq!(stream.converter.state.json_mode, Some(true));
    assert_eq!(stream.converter.state.speed.as_deref(), Some("fast"));
    assert_eq!(&stream.converter.state.tool_name_reverse_map, &reverse);
    assert_eq!(stream.accumulated_bytes, 0);
    assert!(!stream.source_ended);
    // the call runs the hook after the iterator is built, with no extra call from the caller
    assert_post_call(&logger.state(), &messages, "lines", DATA);
}

async fn sync_stream_post(supplied: bool) {
    let messages = messages();
    let reverse = reverse_map();
    let supplied_client = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let module_async = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let module_blocking = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: supplied.then_some(&supplied_client),
        module_level_aclient: &module_async,
        module_level_client: &module_blocking,
        logger: &logger,
    };
    let call = stream_call(DATA, &messages, &reverse);
    let opened = StreamOpening {
        clients,
        call: &call,
        build: stream_build(&FixedIds),
    }
    .make_sync_call()
    .await
    .expect("a 200 reply opens the stream");
    let (served, idle) = if supplied {
        (&supplied_client, [&module_async, &module_blocking])
    } else {
        (&module_blocking, [&supplied_client, &module_async])
    };
    assert_served(served, &idle);
    assert!(opened.sync_stream);
    assert_eq!(
        &opened
            .completion_stream
            .converter
            .state
            .tool_name_reverse_map,
        &reverse
    );
    assert_post_call(
        &logger.state(),
        &messages,
        "first stream response received",
        DATA,
    );
}

fn assert_served(served: &FixtureTransport, idle: &[&FixtureTransport]) {
    for client in idle {
        assert!(client.posts().is_empty());
        assert!(!client.polled());
    }
    assert!(!served.polled());
    let posts = served.posts();
    let post = posts
        .first()
        .expect("the selected client receives the post");
    assert_eq!(post.url.as_str(), API_BASE);
    assert_eq!(post.body, DATA.as_bytes());
    assert!(post.stream);
    assert_eq!(post.deadline, TIMEOUT);
    assert_eq!(header(&post.headers, "x-api-key"), Some(SECRET));
    assert_eq!(
        header(&post.headers, "content-type"),
        Some("application/json")
    );
}

async fn handshake_error() {
    let response = FixtureTransport::status_bytes(
        400,
        ERROR_TEXT.as_bytes().to_vec(),
        response_headers(),
        None,
    );
    let error = handshake_result(&response, false).await;
    let (status, body, headers) = anthropic_error(error);
    assert_eq!(status, 400);
    assert_eq!(body, ERROR_TEXT);
    assert_eq!(header(&headers, "x-request-id"), Some("req-1"));
    // getattr(e, "headers", None) wins over the response's headers when the exception carries its own
    let carried = FixtureTransport::status_bytes(
        400,
        ERROR_TEXT.as_bytes().to_vec(),
        response_headers(),
        Some(exception_headers()),
    );
    let carried_error = handshake_result(&carried, false).await;
    let (carried_status, _, carried_headers) = anthropic_error(carried_error);
    assert_eq!(carried_status, 400);
    assert_eq!(header(&carried_headers, "x-request-id"), Some("req-2"));
    let sync_response = FixtureTransport::status_bytes(
        400,
        ERROR_TEXT.as_bytes().to_vec(),
        response_headers(),
        None,
    );
    let sync_error = handshake_result(&sync_response, true).await;
    let (sync_status, sync_body, sync_headers) = anthropic_error(sync_error);
    assert_eq!(sync_status, 400);
    assert_eq!(sync_body, ERROR_TEXT);
    assert_eq!(header(&sync_headers, "x-request-id"), Some("req-1"));
    // a body that is not UTF-8 reads its byte count, the text a JSON record can carry
    let bytes = FixtureTransport::status_bytes(400, vec![0xff, 0xfe], response_headers(), None);
    let bytes_error = handshake_result(&bytes, false).await;
    let (_, bytes_body, _) = anthropic_error(bytes_error);
    assert_eq!(bytes_body, "2 bytes that are not text");
}

async fn handshake_result(
    transport: &FixtureTransport,
    synchronous: bool,
) -> StreamOpenError<FixtureFailure> {
    let messages = messages();
    let reverse = reverse_map();
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(transport),
        module_level_aclient: transport,
        module_level_client: transport,
        logger: &logger,
    };
    let call = stream_call(DATA, &messages, &reverse);
    let result = if synchronous {
        StreamOpening {
            clients,
            call: &call,
            build: stream_build(&FixedIds),
        }
        .make_sync_call()
        .await
    } else {
        StreamOpening {
            clients,
            call: &call,
            build: stream_build(&FixedIds),
        }
        .make_call()
        .await
    };
    assert_no_post_call(&logger.state());
    let Err(error) = result else {
        panic!("a status error opens no stream")
    };
    error
}

fn anthropic_error<Failure>(error: StreamOpenError<Failure>) -> (u16, String, HeaderMap)
where
    Failure: StdError + Send + Sync + 'static,
{
    match error {
        StreamOpenError::Anthropic {
            status_code,
            message,
            headers,
        } => (
            status_code,
            message,
            headers.expect("the failed reply carries headers"),
        ),
        StreamOpenError::Reraised { .. }
        | StreamOpenError::Unknown { .. }
        | StreamOpenError::Endpoint { .. } => {
            panic!("a status error becomes AnthropicError")
        }
    }
}

async fn non_http_error_500() {
    let async_500 = failed_call(PostFailure::Unknown(FixtureFailure::Unknown), false).await;
    let StreamOpenError::Unknown {
        source: async_source,
    } = async_500
    else {
        panic!("an unrecognized exception becomes AnthropicError(500)")
    };
    assert_eq!(async_source.to_string(), "the transport exploded");
    let blocking_500 = failed_call(PostFailure::Unknown(FixtureFailure::Unknown), true).await;
    let StreamOpenError::Unknown {
        source: blocking_source,
    } = blocking_500
    else {
        panic!("an unrecognized exception becomes AnthropicError(500)")
    };
    assert_eq!(blocking_source.to_string(), "the transport exploded");
    let async_reraised = failed_call(PostFailure::Known(FixtureFailure::Timeout), false).await;
    let StreamOpenError::Reraised {
        source: async_timeout,
    } = async_reraised
    else {
        panic!("a LITELLM_EXCEPTION_TYPES member is re-raised")
    };
    assert!(matches!(async_timeout, FixtureFailure::Timeout));
    let blocking_reraised = failed_call(PostFailure::Known(FixtureFailure::Timeout), true).await;
    let StreamOpenError::Reraised {
        source: blocking_timeout,
    } = blocking_reraised
    else {
        panic!("a LITELLM_EXCEPTION_TYPES member is re-raised")
    };
    assert!(matches!(blocking_timeout, FixtureFailure::Timeout));
}

async fn failed_call(
    failure: PostFailure<FixtureFailure>,
    sync: bool,
) -> StreamOpenError<FixtureFailure> {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::failure(failure);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let call = stream_call(DATA, &messages, &reverse);
    let result = if sync {
        StreamOpening {
            clients,
            call: &call,
            build: stream_build(&FixedIds),
        }
        .make_sync_call()
        .await
    } else {
        StreamOpening {
            clients,
            call: &call,
            build: stream_build(&FixedIds),
        }
        .make_call()
        .await
    };
    assert_no_post_call(&logger.state());
    let Err(error) = result else {
        panic!("a failing transport opens no stream")
    };
    error
}

async fn sync_status_guard() {
    async_status_opens().await;
    sync_status_rejects().await;
    sync_status_rejects_body_over_limit().await;
}

async fn async_status_opens() {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::reply(529, response_headers(), &[ERROR_TEXT]);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let call = stream_call(DATA, &messages, &reverse);
    let opened = StreamOpening {
        clients,
        call: &call,
        build: stream_build(&FixedIds),
    }
    .make_call()
    .await
    .expect("the async helper opens the stream whatever the status");
    assert!(!opened.sync_stream);
    assert!(!transport.polled());
    assert_post_call(&logger.state(), &messages, "lines", DATA);
}

async fn sync_status_rejects() {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::reply(529, response_headers(), &[ERROR_TEXT]);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let Err(error) = StreamOpening {
        clients,
        call: &stream_call(DATA, &messages, &reverse),
        build: stream_build(&FixedIds),
    }
    .make_sync_call()
    .await
    else {
        panic!("the sync helper rejects a returned status")
    };
    assert_no_post_call(&logger.state());
    let (status, body, headers) = anthropic_error(error);
    assert_eq!(status, 529);
    assert_eq!(body, ERROR_TEXT);
    assert_eq!(header(&headers, "x-request-id"), Some("req-1"));
    assert!(transport.polled());
}

async fn sync_status_rejects_body_over_limit() {
    let messages = messages();
    let first = "x".repeat(20);
    let second = "y".repeat(20);
    let lines = [first.as_str(), second.as_str()];
    // a limit that fits the first line exactly keeps it, and the next line no longer fits
    assert_eq!(guard_message(&messages, &lines, 20).await, first);
    // the second line would pass a 32 byte limit, so the message stops after the first whole line
    assert_eq!(guard_message(&messages, &lines, 32).await, first);
    // a limit that fits both lines and their separator keeps both
    assert_eq!(
        guard_message(&messages, &lines, 41).await,
        format!("{first}\n{second}")
    );
}

async fn guard_message(messages: &Json, lines: &[&str], limit: usize) -> String {
    let reverse = reverse_map();
    let transport = FixtureTransport::reply(201, response_headers(), lines);
    let logger = RecordingLogger::new(messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let Err(error) = StreamOpening {
        clients,
        call: &stream_call(DATA, messages, &reverse),
        build: stream_build_with(&FixedIds, limit),
    }
    .make_sync_call()
    .await
    else {
        panic!("the sync helper rejects a returned status")
    };
    assert_no_post_call(&logger.state());
    let (status, body, headers) = anthropic_error(error);
    assert_eq!(status, 201);
    assert_eq!(header(&headers, "x-request-id"), Some("req-1"));
    body
}

async fn stream_flag_and_context() {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let optional = optional_params();
    let params = litellm_params(&reverse);
    let logger = RecordingLogger::new(&messages);
    let wrapper = open_wrapper(&transport, &logger, &messages, &optional, &params).await;
    assert_eq!(wrapper.model, "claude-3-5-sonnet");
    assert_eq!(wrapper.custom_llm_provider, "anthropic");
    let state = &wrapper.completion_stream.converter.state;
    assert_eq!(state.json_mode, Some(false));
    assert_eq!(state.speed.as_deref(), Some("fast"));
    assert_eq!(&state.tool_name_reverse_map, &reverse);
    let posts = transport.posts();
    let post = posts.first().expect("the wrapper posts once");
    let body = String::from_utf8(post.body.clone()).expect("the posted body is JSON text");
    // stream=True replaces the key the caller sent, so it keeps its place in the dict json.dumps writes
    assert_eq!(
        body,
        r#"{"model": "claude-3-5-sonnet", "stream": true, "messages": [{"role": "user", "content": "hi"}]}"#
    );
    // the wrapper reaches post_call through the call it makes, as LiteLLM's acompletion_stream_function does
    assert_post_call(&logger.state(), &messages, "lines", &body);
}

async fn response_headers_case() {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let opened = StreamOpening {
        clients,
        call: &stream_call(DATA, &messages, &reverse),
        build: stream_build(&FixedIds),
    }
    .make_call()
    .await
    .expect("a 200 reply opens the stream");
    assert_eq!(header(&opened.headers, "x-request-id"), Some("req-1"));
    assert_eq!(
        header(&opened.headers, "anthropic-ratelimit-requests-limit"),
        Some("100")
    );
    projected_headers().await;
}

async fn projected_headers() {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let logger = RecordingLogger::new(&messages);
    let optional = optional_params();
    let params = litellm_params(&reverse);
    let wrapper = open_wrapper(&transport, &logger, &messages, &optional, &params).await;
    let projected = &wrapper.response_headers;
    assert_eq!(
        projected
            .get("llm_provider-x-request-id")
            .map(String::as_str),
        Some("req-1")
    );
    assert_eq!(
        projected
            .get("llm_provider-anthropic-ratelimit-requests-limit")
            .map(String::as_str),
        Some("100")
    );
    assert_eq!(
        projected
            .get("x-ratelimit-limit-requests")
            .map(String::as_str),
        Some("100")
    );
}

fn optional_params() -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert("speed".to_owned(), Json::Text("fast".to_owned()));
    params
}

fn litellm_params(reverse: &IndexMap<String, String>) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    for (sanitized, original) in reverse {
        fields.insert(sanitized.clone(), Json::Text(original.clone()));
    }
    let mut params = IndexMap::new();
    params.insert(REVERSE_MAP_KEY.to_owned(), Json::Object(fields));
    params
}

fn stream_data(messages: &Json) -> IndexMap<String, Json> {
    let mut data = IndexMap::new();
    data.insert(
        "model".to_owned(),
        Json::Text("claude-3-5-sonnet".to_owned()),
    );
    data.insert("stream".to_owned(), Json::Bool(false));
    data.insert("messages".to_owned(), messages.clone());
    data
}

fn completion<'call>(
    messages: &'call Json,
    data: IndexMap<String, Json>,
    optional_params: &'call IndexMap<String, Json>,
    litellm_params: &'call IndexMap<String, Json>,
) -> CompletionStream<'call> {
    CompletionStream {
        model: "claude-3-5-sonnet",
        api_base: API_BASE,
        headers: request_headers(),
        data,
        messages,
        timeout: TIMEOUT,
        json_mode: false,
        optional_params,
        litellm_params,
    }
}

async fn open_wrapper(
    transport: &FixtureTransport,
    logger: &RecordingLogger,
    messages: &Json,
    optional: &IndexMap<String, Json>,
    params: &IndexMap<String, Json>,
) -> WrappedStream<FixtureFailure, FixedCounter> {
    let clients = StreamClients {
        supplied: Some(transport),
        module_level_aclient: transport,
        module_level_client: transport,
        logger,
    };
    acompletion_stream_function(
        clients,
        completion(messages, stream_data(messages), optional, params),
        stream_build(&FixedIds),
    )
    .await
    .expect("the fixture reply opens the wrapper")
}

// the api_base is parsed before the request is built, so a malformed one fails as the typed Endpoint error
#[tokio::test]
async fn anthropic_http_stream_malformed_api_base() {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let logger = RecordingLogger::new(&messages);
    let call = StreamCall {
        api_base: "not a url",
        headers: request_headers(),
        data: DATA,
        messages: &messages,
        timeout: TIMEOUT,
        json_mode: false,
        speed: None,
        tool_name_reverse_map: &reverse,
    };
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let Err(error) = StreamOpening {
        clients,
        call: &call,
        build: stream_build(&FixedIds),
    }
    .make_call()
    .await
    else {
        panic!("a malformed api_base opens no stream")
    };
    assert!(matches!(error, StreamOpenError::Endpoint { .. }));
    assert!(transport.posts().is_empty());
    assert_no_post_call(&logger.state());
}
