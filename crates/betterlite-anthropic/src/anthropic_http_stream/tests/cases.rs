#![cfg(test)]

use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;

use crate::anthropic_http_stream::call::{StreamCall, StreamClients, make_call, make_sync_call};
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

#[rstest]
#[case::case_0_stream_post(0)]
#[case::case_1_handshake_error(1)]
#[case::case_2_non_http_error_500(2)]
#[case::case_3_sync_status_guard(3)]
#[case::case_4_stream_flag_and_context(4)]
#[case::case_5_response_headers(5)]
#[tokio::test]
async fn anthropic_http_stream_cases(#[case] case: u8) {
    match case {
        0 => stream_post().await,
        1 => handshake_error().await,
        2 => non_http_error_500().await,
        3 => sync_status_guard().await,
        4 => stream_flag_and_context().await,
        _ => response_headers().await,
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
    let module_sync = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: supplied.then_some(&supplied_client),
        module_level_aclient: &module_async,
        module_level_client: &module_sync,
        logger: &logger,
    };
    let call = stream_call(DATA, &messages, &reverse);
    let opened = make_call(clients, &call, stream_build(&FixedIds))
        .await
        .expect("a 200 reply opens the stream");
    assert_served(supplied, &supplied_client, &module_async, &module_sync);
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
    let module_sync = FixtureTransport::reply(200, response_headers(), &[FRAME]);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: supplied.then_some(&supplied_client),
        module_level_aclient: &module_async,
        module_level_client: &module_sync,
        logger: &logger,
    };
    let call = stream_call(DATA, &messages, &reverse);
    let opened = make_sync_call(clients, &call, stream_build(&FixedIds))
        .await
        .expect("a 200 reply opens the stream");
    assert_served(supplied, &supplied_client, &module_async, &module_sync);
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

fn assert_served(
    supplied: bool,
    supplied_client: &FixtureTransport,
    module_async: &FixtureTransport,
    module_sync: &FixtureTransport,
) {
    let (served, untouched) = if supplied {
        (supplied_client.posts(), module_async.posts())
    } else {
        (module_async.posts(), supplied_client.posts())
    };
    assert!(untouched.is_empty());
    assert!(module_sync.posts().is_empty());
    assert!(!supplied_client.polled());
    assert!(!module_async.polled());
    let post = served
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
    let error = async_handshake(None).await;
    let (status, body, headers) = anthropic_error(error);
    assert_eq!(status, 400);
    assert_eq!(body, ERROR_TEXT);
    assert_eq!(header(&headers, "x-request-id"), Some("req-1"));
    // getattr(e, "headers", None) wins over the response's headers when the exception carries its own
    let error = async_handshake(Some(exception_headers())).await;
    let (status, _, headers) = anthropic_error(error);
    assert_eq!(status, 400);
    assert_eq!(header(&headers, "x-request-id"), Some("req-2"));
    let error = sync_handshake().await;
    let (status, body, headers) = anthropic_error(error);
    assert_eq!(status, 400);
    assert_eq!(body, ERROR_TEXT);
    assert_eq!(header(&headers, "x-request-id"), Some("req-1"));
    // a body that is not UTF-8 reads its byte count, the text a JSON record can carry
    let error = undecodable_handshake().await;
    let (status, body, _headers) = anthropic_error(error);
    assert_eq!(status, 400);
    assert_eq!(body, "2 bytes that are not text");
}

async fn async_handshake(exception_headers: Option<HeaderMap>) -> StreamOpenError<FixtureFailure> {
    let messages = messages();
    let reverse = reverse_map();
    let transport =
        FixtureTransport::status(400, ERROR_TEXT, response_headers(), exception_headers);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let result = make_call(
        clients,
        &stream_call(DATA, &messages, &reverse),
        stream_build(&FixedIds),
    )
    .await;
    assert_no_post_call(&logger.state());
    match result {
        Ok(_) => panic!("a status error opens no stream"),
        Err(error) => error,
    }
}

async fn sync_handshake() -> StreamOpenError<FixtureFailure> {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::status(400, ERROR_TEXT, response_headers(), None);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let result = make_sync_call(
        clients,
        &stream_call(DATA, &messages, &reverse),
        stream_build(&FixedIds),
    )
    .await;
    assert_no_post_call(&logger.state());
    match result {
        Ok(_) => panic!("a status error opens no stream"),
        Err(error) => error,
    }
}

async fn undecodable_handshake() -> StreamOpenError<FixtureFailure> {
    let messages = messages();
    let reverse = reverse_map();
    let transport = FixtureTransport::status_bytes(400, vec![0xff, 0xfe], response_headers(), None);
    let logger = RecordingLogger::new(&messages);
    let clients = StreamClients {
        supplied: Some(&transport),
        module_level_aclient: &transport,
        module_level_client: &transport,
        logger: &logger,
    };
    let result = make_call(
        clients,
        &stream_call(DATA, &messages, &reverse),
        stream_build(&FixedIds),
    )
    .await;
    assert_no_post_call(&logger.state());
    match result {
        Ok(_) => panic!("a status error opens no stream"),
        Err(error) => error,
    }
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
    let error = failed_call(PostFailure::Unknown(FixtureFailure::Unknown), false).await;
    let StreamOpenError::Unknown { source } = error else {
        panic!("an unrecognized exception becomes AnthropicError(500)")
    };
    assert_eq!(source.to_string(), "the transport exploded");
    let error = failed_call(PostFailure::Unknown(FixtureFailure::Unknown), true).await;
    let StreamOpenError::Unknown { source } = error else {
        panic!("an unrecognized exception becomes AnthropicError(500)")
    };
    assert_eq!(source.to_string(), "the transport exploded");
    let error = failed_call(PostFailure::Known(FixtureFailure::Timeout), false).await;
    let StreamOpenError::Reraised { source } = error else {
        panic!("a LITELLM_EXCEPTION_TYPES member is re-raised")
    };
    assert!(matches!(source, FixtureFailure::Timeout));
    let error = failed_call(PostFailure::Known(FixtureFailure::Timeout), true).await;
    let StreamOpenError::Reraised { source } = error else {
        panic!("a LITELLM_EXCEPTION_TYPES member is re-raised")
    };
    assert!(matches!(source, FixtureFailure::Timeout));
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
        make_sync_call(clients, &call, stream_build(&FixedIds)).await
    } else {
        make_call(clients, &call, stream_build(&FixedIds)).await
    };
    assert_no_post_call(&logger.state());
    match result {
        Ok(_) => panic!("a failing transport opens no stream"),
        Err(error) => error,
    }
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
    let opened = make_call(clients, &call, stream_build(&FixedIds))
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
    let error = match make_sync_call(
        clients,
        &stream_call(DATA, &messages, &reverse),
        stream_build(&FixedIds),
    )
    .await
    {
        Ok(_) => panic!("the sync helper rejects a returned status"),
        Err(error) => error,
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
    let error = match make_sync_call(
        clients,
        &stream_call(DATA, messages, &reverse),
        stream_build_with(&FixedIds, limit),
    )
    .await
    {
        Ok(_) => panic!("the sync helper rejects a returned status"),
        Err(error) => error,
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

async fn response_headers() {
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
    let opened = make_call(
        clients,
        &stream_call(DATA, &messages, &reverse),
        stream_build(&FixedIds),
    )
    .await
    .expect("a 200 reply opens the stream");
    assert_eq!(header(&opened.headers, "x-request-id"), Some("req-1"));
    assert_eq!(
        header(&opened.headers, "anthropic-ratelimit-requests-limit"),
        Some("100")
    );
    let transport = FixtureTransport::reply(200, response_headers(), &[FRAME]);
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
    let error = match make_call(clients, &call, stream_build(&FixedIds)).await {
        Ok(_) => panic!("a malformed api_base opens no stream"),
        Err(error) => error,
    };
    assert!(matches!(error, StreamOpenError::Endpoint { .. }));
    assert!(transport.posts().is_empty());
    assert_no_post_call(&logger.state());
}
