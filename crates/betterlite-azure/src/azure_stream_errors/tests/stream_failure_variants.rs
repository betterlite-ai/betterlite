use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::azure_stream_errors::error::{Raised, Reported};
use crate::azure_stream_errors::prepare::ASYNC_CLIENT_MESSAGE;
use crate::azure_stream_errors::records::StreamCall;
use crate::azure_stream_errors::{async_streaming, streaming};
use crate::endpoints::projection::ClientKind;

use super::fixture::{
    CallFixture, Recorder, ScriptedSource, ScriptedSources, azure_failure, block_on,
    failing_source, runtime, scripted_source, secret_headers, sse_payload,
};

pub(super) fn assert_sync_original_class(
    sources: &ScriptedSources,
    call: &StreamCall<'_>,
    recorder: &mut Recorder,
) {
    let source = failing_source(
        ClientKind::AzureOpenAi,
        Raised::plain("socket closed".to_owned()),
    );
    let error = block_on(streaming(&source, sources, call, recorder, runtime()))
        .err()
        .expect("sync keeps the original class");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "Exception");
    assert_eq!(failure.status, Reported::Absent);
    assert_eq!(failure.text, "socket closed");
}

pub(super) fn assert_response_fallback(call: &StreamCall<'_>, recorder: &mut Recorder) {
    let fallback = secret_headers(&[("retry-after", "9")]);
    let mut response_only = Raised::plain("sent".to_owned());
    response_only.response_headers = Some(fallback.clone());
    let source = failing_source(ClientKind::AsyncAzureOpenAi, response_only);
    let failure = azure_failure(
        block_on(async_streaming(&source, call, recorder, runtime()))
            .err()
            .expect("response headers fall back"),
    );
    assert_eq!(failure.headers, Some(fallback));
}

pub(super) fn assert_asked_stream_options(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    recorder: &mut Recorder,
) {
    let mut asked = CallFixture::default();
    asked.data.insert(
        "stream_options".to_owned(),
        Json::Object(IndexMap::from([(
            "include_usage".to_owned(),
            Json::Bool(true),
        )])),
    );
    let asked_call = asked.call();
    let asked_stream = block_on(streaming(source, sources, &asked_call, recorder, runtime()))
        .expect("asked stream options open");
    assert_eq!(
        asked_stream
            .stream_options
            .as_ref()
            .map(|options| options.include_usage),
        Some(true)
    );
}

pub(super) fn assert_async_refuses_sync(call: &StreamCall<'_>, recorder: &mut Recorder) {
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let error = block_on(async_streaming(&source, call, recorder, runtime()))
        .err()
        .expect("async refuses a sync client");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(500)));
    assert_eq!(
        failure.message,
        Reported::Present(Some(ASYNC_CLIENT_MESSAGE.to_owned()))
    );
    assert!(recorder.pre.is_empty());
}
