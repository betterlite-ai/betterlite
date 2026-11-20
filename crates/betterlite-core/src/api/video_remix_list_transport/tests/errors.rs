use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;

use super::recording::{RecordingConfig, Step};
use super::support::{
    Bench, CallKind, CallMode, Events, IMPATIENT, ProxiedSource, Setup, json_params, json_server,
    proxied_bench, run,
};
use crate::api::video_remix_list_transport::client::VideoClientSource;
use crate::api::video_remix_list_transport::error::{
    ErrorHeaders, Operation, VideoRemixListTransportError,
};

pub(super) const REMIX: &str = "video_remix";
pub(super) const LIST: &str = "video_list";
pub(super) const PROMPT: &str = "make it slow";
pub(super) const CLOSED: &str = "the client is shutting down and takes no new requests";
pub(super) const DEADLINE: &str = "the request ran past its read timeout or total deadline";

#[derive(Debug, Clone, Copy)]
pub(super) enum ErrorCase {
    RemixStatus,
    RemixStatusEmptyBody,
    RemixStatusBinaryBody,
    RemixSendFailure,
    RemixTimeout,
    RemixCallerTimeout,
    SyncTimeoutText,
    RemixResponseConversion,
    RemixRequestConversion,
    RemixUrlFailure,
    ListStatus,
    ListSendFailure,
    ListTimeout,
    ListResponseConversion,
    ListQueryObject,
    ListQueryNested,
    UnknownProvider,
    PoolFailure,
    ApiBaseType,
    ApiKeyType,
    SslVerifyType,
    ListSslVerifyType,
    ListApiBaseType,
}

pub(super) fn provider(error: &VideoRemixListTransportError) -> &ProviderError {
    let VideoRemixListTransportError::Provider(provider) = error else {
        panic!("the handler answers with the provider's error, not {error}");
    };
    provider
}

pub(super) fn expected_headers(pairs: &[(&str, &str)]) -> ErrorHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

// A failure that went through the conversion keeps the status, the message and the config's error class
pub(super) fn assert_error<Source>(
    bench: &Bench<Source>,
    error: &VideoRemixListTransportError,
    status: u16,
    message: &str,
    synthesized: bool,
) where
    Source: VideoClientSource,
{
    let provider = provider(error);
    assert_eq!(provider.status_code, status);
    assert_eq!(provider.status_code_is_synthesized, synthesized);
    assert_eq!(provider.message, message);
    let calls = bench.config.errors();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status_code, status);
    assert_eq!(calls[0].message, message);
    if synthesized {
        assert!(calls[0].headers.is_empty());
    }
}

pub(super) fn assert_prepare(
    error: &VideoRemixListTransportError,
    expected: Operation,
    step: Step,
) {
    let VideoRemixListTransportError::Prepare { operation, source } = error else {
        panic!("the failure is a prepare failure, not {error}");
    };
    assert_eq!(*operation, expected);
    assert_eq!(
        error.to_string(),
        format!("the video {expected} request could not be prepared")
    );
    assert_eq!(
        source.source.to_string(),
        format!("the fake config fails at {step:?}")
    );
}

pub(super) fn assert_field(error: &VideoRemixListTransportError, message: &str) {
    assert!(matches!(error, VideoRemixListTransportError::Field(_)));
    assert_eq!(error.to_string(), message);
}

pub(super) fn assert_outside_transform<Source>(bench: &Bench<Source>, transform: &str)
where
    Source: VideoClientSource,
{
    let events = bench.config.events();
    let wanted = transform.to_owned();
    assert_eq!(events.last().map(String::as_str), Some("error_class"));
    assert!(!events.contains(&wanted));
}

// The async remix timeout against the real transport, so the send itself produces the deadline
pub(super) async fn timed_remix(
    timeout: Option<Duration>,
) -> (Bench<ProxiedSource>, VideoRemixListTransportError) {
    let serving = json_server("{}", Duration::from_millis(300));
    let address = serving.address;
    let configure = move |events: &Events| {
        RecordingConfig::new(events)
            .with_base(&format!("http://{address}/v1"))
            .with_remix_url(&format!("http://{address}/v1/videos/video_a/remix"))
    };
    let mut bench = proxied_bench(REMIX, configure, IMPATIENT);
    let params = json_params("{}");
    let mut setup = Setup::new("openai", &params).video("video_a", PROMPT);
    setup.timeout = timeout;
    let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
        .await
        .expect_err("the read timeout is a remix timeout");
    serving.finish().expect("the fixture finishes");
    (bench, error)
}

pub(super) fn assert_timeout(
    bench: &Bench<ProxiedSource>,
    error: &VideoRemixListTransportError,
    passed: &str,
) {
    let provider = provider(error);
    let prefix =
        format!("litellm.Timeout: Connection timed out. Timeout passed={passed}, time taken=");
    assert_eq!(provider.status_code, 408);
    assert!(!provider.status_code_is_synthesized);
    assert!(
        provider.message.starts_with(&prefix),
        "{}",
        provider.message
    );
    assert!(
        provider.message.ends_with(" seconds"),
        "{}",
        provider.message
    );
    let calls = bench.config.errors();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status_code, 408);
    assert_eq!(calls[0].message, provider.message);
    assert!(calls[0].headers.is_empty());
    assert_outside_transform(bench, "remix_response");
}
