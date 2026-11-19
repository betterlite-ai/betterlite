use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use crate::azure_stream_errors::error::{AzureStreamFailure, Raised, Reported, StreamingError};
use crate::azure_stream_errors::stream::raw_headers;
use crate::azure_stream_errors::{async_streaming, streaming};
use crate::endpoints::projection::ClientKind;

use super::fixture::{
    CallFixture, Recorder, ScriptedSources, azure_failure, block_on, failing_source, raised_with,
    runtime, scripted_source, secret_headers, sse_payload,
};

#[test]
fn azure_chat_streams_credential_identity_isolation() {
    let assertions = BTreeMap::from([
        (
            "env/ASSERTION-A".to_owned(),
            SecretString::from("assertion-A"),
        ),
        (
            "env/ASSERTION-B".to_owned(),
            SecretString::from("assertion-B"),
        ),
    ]);
    let sources = ScriptedSources::echoing(assertions);
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());

    let mut first = CallFixture::default();
    first.api_key = None;
    first.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION-A"));
    let first_call = first.call();

    let mut second = CallFixture::default();
    second.api_key = None;
    second.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION-B"));
    let second_call = second.call();

    let mut recorder = Recorder::default();
    block_on(streaming(
        &source,
        &sources,
        &first_call,
        &mut recorder,
        runtime(),
    ))
    .expect("first identity opens");
    block_on(streaming(
        &source,
        &sources,
        &second_call,
        &mut recorder,
        runtime(),
    ))
    .expect("second identity opens");
    block_on(streaming(
        &source,
        &sources,
        &first_call,
        &mut recorder,
        runtime(),
    ))
    .expect("first identity opens again");

    let tokens: Vec<Json> = recorder
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
        .collect();
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

    let error = StreamingError::Azure(failed);
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
