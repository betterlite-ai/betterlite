use std::time::Duration;

use betterlite_http::interactions::base::RequestFailure;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_logging::logging_state::LoggingState;

use super::support::{
    CHARACTER_BODY, Events, LogEnv, RecordingConfig, Seen, TestError, create_call, extra_headers,
    get_call, logging_state, params, real_clients, recording, secret, video,
};
use crate::api::video_character_transport::error::CharacterError;
use crate::api::video_character_transport::service::{CharacterHandled, CharacterTransport};

fn parts(call_type: &str) -> (RecordingConfig, LogEnv, LoggingState, HttpClients) {
    let config = recording(&Events::new());
    let log = LogEnv::new(&Events::new());
    (config, log, logging_state(call_type), real_clients())
}

fn assert_get_reply_seen(config: &RecordingConfig) {
    assert_eq!(
        config.seen().last(),
        Some(&Seen::GetReply {
            status: 200,
            body: CHARACTER_BODY.as_bytes().to_vec(),
        })
    );
}

#[test]
fn creation_writes_the_multipart_body_and_headers_to_the_wire() {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_create_character");
    config.create_url = format!("{base}/v1/characters");
    config
        .environment
        .insert("authorization".to_owned(), secret("Bearer sk-provider"));
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    transport.boundary = || String::from("fixed-boundary");
    let litellm = params(&[("api_base", base.as_str())]);
    let caller_headers = extra_headers(&[("x-caller", "yes")]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.extra_headers = Some(&caller_headers);

    let object = match transport.video_create_character_handler(&call) {
        Ok(CharacterHandled::Done(object)) => object,
        Ok(CharacterHandled::Pending(_)) => panic!("a sync call answers in place"),
        Err(error) => panic!("the creation reaches the server: {error:?}"),
    };

    assert_eq!(object, config.object);
    let request = serving.finish().unwrap();
    assert_eq!(request.line, "POST /v1/characters HTTP/1.1");
    assert!(
        request
            .head
            .contains("content-type: multipart/form-data; boundary=fixed-boundary")
    );
    assert!(request.head.contains("x-caller: yes"));
    assert!(request.head.contains("authorization: Bearer sk-provider"));
    let expected = String::from(
        "--fixed-boundary\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nWren\r\n--fixed-boundary\r\nContent-Disposition: form-data; name=\"video\"; filename=\"clip.mp4\"\r\nContent-Type: video/mp4\r\n\r\nvideo-bytes\r\n--fixed-boundary--\r\n",
    );
    assert_eq!(request.body, expected);
}

#[test]
fn creation_forwards_its_timeout_to_the_post() {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::from_millis(300)).unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_create_character");
    config.create_url = format!("{base}/v1/characters");
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let litellm = params(&[("api_base", base.as_str())]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.timeout = Some(Duration::from_millis(50));

    let error = match transport.video_create_character_handler(&call) {
        Ok(_handled) => panic!("the deadline ends the call"),
        Err(error) => error,
    };

    let CharacterError::Transport(RequestFailure::Transport(source)) = error else {
        panic!("the timeout surfaces as a transport failure");
    };
    assert!(source.is_timeout());
    drop(serving.finish());
}

#[test]
fn creation_without_a_timeout_waits_for_a_slow_reply() {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::from_millis(150)).unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_create_character");
    config.create_url = format!("{base}/v1/characters");
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let litellm = params(&[("api_base", base.as_str())]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let object = match transport.video_create_character_handler(&call) {
        Ok(CharacterHandled::Done(object)) => object,
        Ok(CharacterHandled::Pending(_)) => panic!("a sync call answers in place"),
        Err(error) => panic!("an absent timeout leaves the wire deadline: {error:?}"),
    };

    assert_eq!(object, config.object);
    serving.finish().unwrap();
}

#[test]
fn lookup_sends_a_get_with_its_params_and_ignores_the_calls_timeout() {
    let serving = serve(
        reply("200 OK", &[], CHARACTER_BODY),
        Duration::from_millis(150),
    )
    .unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_get_character");
    config.get_url = format!("{base}/v1/characters/char-1?expand=url");
    config.query = vec![
        (String::from("expand"), String::from("param")),
        (String::from("view"), String::from("full")),
    ];
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let litellm = params(&[("api_base", base.as_str())]);
    let mut call = get_call("char-1", &litellm);
    call.timeout = Some(Duration::from_millis(50));

    let object = match transport.video_get_character_handler(&call) {
        Ok(CharacterHandled::Done(object)) => object,
        Ok(CharacterHandled::Pending(_)) => panic!("a sync call answers in place"),
        Err(error) => panic!("the lookup ignores the send timeout: {error:?}"),
    };

    assert_eq!(object, config.object);
    assert_get_reply_seen(&config);
    let request = serving.finish().unwrap();
    assert_eq!(
        request.line,
        "GET /v1/characters/char-1?expand=url&view=full HTTP/1.1"
    );
    assert!(request.body.is_empty());
}

#[test]
fn lookup_turns_a_real_status_into_the_provider_error() {
    let serving = serve(
        reply(
            "400 Bad Request",
            &[("retry-after", "5")],
            "no such character",
        ),
        Duration::ZERO,
    )
    .unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_get_character");
    config.get_url = format!("{base}/v1/characters/char-1");
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let litellm = params(&[("api_base", base.as_str())]);
    let call = get_call("char-1", &litellm);

    let error = match transport.video_get_character_handler(&call) {
        Ok(_handled) => panic!("a 400 ends the call"),
        Err(error) => error,
    };

    let CharacterError::Provider(TestError {
        message,
        status_code,
        headers,
    }) = error
    else {
        panic!("the real reply becomes the provider's error");
    };
    assert_eq!(message, "no such character");
    assert_eq!(status_code, 400);
    assert_eq!(headers.get("retry-after").map(String::as_str), Some("5"));
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_creation_writes_the_multipart_body_to_the_wire() {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_create_character");
    config.create_url = format!("{base}/v1/characters");
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    transport.boundary = || String::from("async-boundary");
    let litellm = params(&[("api_base", base.as_str())]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let object = transport
        .async_video_create_character_handler(&call)
        .await
        .unwrap();

    assert_eq!(object, config.object);
    let request = serving.finish().unwrap();
    assert_eq!(request.line, "POST /v1/characters HTTP/1.1");
    assert!(
        request
            .head
            .contains("content-type: multipart/form-data; boundary=async-boundary")
    );
    assert!(request.body.contains("name=\"video\""));
    assert!(request.body.contains("video-bytes"));
}

#[tokio::test]
async fn async_creation_forwards_its_timeout_to_the_post() {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::from_millis(300)).unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_create_character");
    config.create_url = format!("{base}/v1/characters");
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let litellm = params(&[("api_base", base.as_str())]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.timeout = Some(Duration::from_millis(50));

    let error = transport
        .async_video_create_character_handler(&call)
        .await
        .unwrap_err();

    let CharacterError::Transport(RequestFailure::Transport(source)) = error else {
        panic!("the timeout surfaces as a transport failure");
    };
    assert!(source.is_timeout());
    drop(serving.finish());
}

#[tokio::test]
async fn async_lookup_reads_the_real_reply() {
    let serving = serve(reply("200 OK", &[], CHARACTER_BODY), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let (mut config, log, mut state, mut clients) = parts("video_get_character");
    config.get_url = format!("{base}/v1/characters/char-1");
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let litellm = params(&[("api_base", base.as_str())]);
    let call = get_call("char-1", &litellm);

    let object = transport
        .async_video_get_character_handler(&call)
        .await
        .unwrap();

    assert_eq!(object, config.object);
    assert_get_reply_seen(&config);
    let request = serving.finish().unwrap();
    assert_eq!(request.line, "GET /v1/characters/char-1 HTTP/1.1");
}
