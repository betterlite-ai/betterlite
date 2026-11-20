use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use crate::azure_stream_errors::error::{AzureStreamFailure, Raised, Reported, StreamingError};
use crate::azure_stream_errors::records::StreamCall;
use crate::azure_stream_errors::stream::raw_headers;
use crate::azure_stream_errors::{async_streaming, streaming};
use crate::endpoints::projection::ClientKind;

use super::fixture::{
    CallFixture, Recorder, ScriptedSource, ScriptedSources, azure_failure, block_on,
    failing_source, raised_with, runtime, scripted_source, secret_headers, sse_payload,
};

fn open_identity_stream(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    call: &StreamCall<'_>,
    recorder: &mut Recorder,
) {
    block_on(streaming(source, sources, call, recorder, runtime()))
        .expect("the identity opens a stream");
}

fn recorded_tokens(recorder: &Recorder) -> Vec<Json> {
    recorder
        .pre
        .iter()
        .map(|call| {
            call.additional_args
                .headers
                .as_ref()
                .and_then(|headers| headers.get("azure_ad_token"))
                .cloned()
                .unwrap_or(Json::Null)
        })
        .collect()
}

fn identity_assertions() -> BTreeMap<String, SecretString> {
    BTreeMap::from([
        (
            "env/ASSERTION-A".to_owned(),
            SecretString::from("assertion-A"),
        ),
        (
            "env/ASSERTION-B".to_owned(),
            SecretString::from("assertion-B"),
        ),
    ])
}

#[test]
fn azure_chat_streams_credential_identity_isolation() {
    let sources = ScriptedSources::echoing(identity_assertions());
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());

    let first = CallFixture {
        api_key: None,
        azure_ad_token: Some(SecretString::from("oidc/env/ASSERTION-A")),
        ..CallFixture::default()
    };
    let first_call = first.call();

    let second = CallFixture {
        api_key: None,
        azure_ad_token: Some(SecretString::from("oidc/env/ASSERTION-B")),
        ..CallFixture::default()
    };
    let second_call = second.call();

    let mut recorder = Recorder::default();
    open_identity_stream(&source, &sources, &first_call, &mut recorder);
    open_identity_stream(&source, &sources, &second_call, &mut recorder);
    open_identity_stream(&source, &sources, &first_call, &mut recorder);

    let tokens = recorded_tokens(&recorder);
    assert_eq!(tokens[0], Json::Text("token-assertion-A".to_owned()));
    assert_eq!(tokens[1], Json::Text("token-assertion-B".to_owned()));
    assert_eq!(tokens[2], tokens[0]);
    assert_ne!(tokens[0], tokens[1]);

    let forms = sources.forms();
    assert_eq!(forms.len(), 2);
    assert!(
        forms[0]
            .1
            .contains(&("client_assertion".to_owned(), "assertion-A".to_owned()))
    );
    assert!(
        forms[1]
            .1
            .contains(&("client_assertion".to_owned(), "assertion-B".to_owned()))
    );
}

#[test]
fn repeated_response_headers_join_with_a_comma() {
    let mut headers = HeaderMap::new();
    headers.append("x-ms-tag", HeaderValue::from_static("first"));
    headers.append("x-ms-tag", HeaderValue::from_static("second"));
    headers.append(
        "content-type",
        HeaderValue::from_static("text/event-stream"),
    );
    let raw = raw_headers(&headers);
    assert_eq!(
        raw.get("x-ms-tag").map(String::as_str),
        Some("first, second")
    );
    assert_eq!(
        raw.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );
}

#[test]
fn streaming_error_display_names_the_azure_message() {
    let failure = AzureStreamFailure::wrapped(Raised::plain("socket closed".to_owned()));
    let error = StreamingError::Azure(Box::new(failure));
    assert_eq!(error.to_string(), "socket closed");
}

#[test]
fn a_missing_messages_key_fails_before_pre_call() {
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let sources = ScriptedSources::new(200, "{}", BTreeMap::new());
    let mut fixture = CallFixture::default();
    fixture.data.shift_remove("messages");
    let call = fixture.call();
    let mut recorder = Recorder::default();
    let error = block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .err()
    .expect("a missing messages key fails");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "KeyError");
    assert_eq!(
        failure.message,
        Reported::Present(Some("'messages'".to_owned()))
    );
    assert!(recorder.pre.is_empty());

    let async_source = scripted_source(ClientKind::AsyncAzureOpenAi, &[], sse_payload());
    let wrapped = block_on(async_streaming(
        &async_source,
        &call,
        &mut recorder,
        runtime(),
    ))
    .err()
    .expect("async wraps the missing key");
    let wrapped_failure = azure_failure(wrapped);
    assert_eq!(wrapped_failure.class, "AzureOpenAIError");
    assert_eq!(wrapped_failure.status, Reported::Present(Some(500)));
    assert_eq!(
        wrapped_failure.message,
        Reported::Present(Some("'messages'".to_owned()))
    );
}

#[test]
fn azure_chat_streams_secret_safe_failure() {
    let secret = "super-secret-client-key";
    let headers = secret_headers(&[("authentication", secret), ("api-key", secret)]);

    let mut failed = AzureStreamFailure::raised(Raised::azure(401, "unauthorized".to_owned()));
    failed.headers = Some(headers.clone());
    let debug = format!("{failed:?}");
    assert!(!debug.contains(secret));

    let mut raised = raised_with(Some(401), "unauthorized".to_owned());
    raised.headers = Some(headers.clone());
    assert!(!format!("{raised:?}").contains(secret));

    let error = StreamingError::Azure(Box::new(failed));
    assert!(!error.to_string().contains(secret));

    let wrapped = AzureStreamFailure::wrapped(raised);
    assert!(!format!("{wrapped:?}").contains(secret));

    let source = failing_source(
        ClientKind::AsyncAzureOpenAi,
        Raised {
            headers: Some(headers),
            ..raised_with(Some(401), "unauthorized".to_owned())
        },
    );
    let fixture = CallFixture::default();
    let call = fixture.call();
    let mut recorder = Recorder::default();
    let stream_error = block_on(async_streaming(&source, &call, &mut recorder, runtime()))
        .err()
        .expect("the wrapped failure is reported");
    assert!(!format!("{stream_error:?}").contains(secret));
    assert!(!stream_error.to_string().contains(secret));
    let stream_failure = azure_failure(stream_error);
    assert_eq!(stream_failure.class, "AzureOpenAIError");
    assert_eq!(stream_failure.status, Reported::Present(Some(401)));
    assert!(
        stream_failure.headers.as_ref().is_some_and(|values| values
            .0
            .get("authentication")
            .map(String::as_str)
            == Some(secret))
    );
}
