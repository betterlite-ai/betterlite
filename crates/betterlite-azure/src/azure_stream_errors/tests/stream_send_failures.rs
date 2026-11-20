use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use indexmap::IndexMap;

use crate::azure_stream_errors::error::{Raised, Reported};
use crate::azure_stream_errors::records::StreamCall;
use crate::azure_stream_errors::{async_streaming, streaming};
use crate::endpoints::projection::ClientKind;

use super::fixture::{
    CallFixture, Recorder, ScriptedSource, ScriptedSources, azure_failure, block_on,
    failing_source, runtime, secret_headers,
};

pub(super) fn assert_send_failure(
    sources: &ScriptedSources,
    call: &StreamCall<'_>,
    recorder: &mut Recorder,
) {
    let sync_source = failing_source(
        ClientKind::AzureOpenAi,
        Raised::azure(400, "bad request".to_owned()),
    );
    let sync_failure = azure_failure(
        block_on(streaming(&sync_source, sources, call, recorder, runtime()))
            .err()
            .expect("sync propagates the send failure"),
    );
    assert_eq!(sync_failure.class, "AzureOpenAIError");
    assert_eq!(sync_failure.status, Reported::Present(Some(400)));
    assert_eq!(
        sync_failure.message,
        Reported::Present(Some("bad request".to_owned()))
    );

    let async_source = failing_source(
        ClientKind::AsyncAzureOpenAi,
        Raised::raised(
            "ScriptedError".to_owned(),
            Reported::Present(None),
            Reported::Present(None),
            "".to_owned(),
        ),
    );
    let async_failure = azure_failure(
        block_on(async_streaming(&async_source, call, recorder, runtime()))
            .err()
            .expect("explicit None stays None"),
    );
    assert_eq!(async_failure.class, "AzureOpenAIError");
    assert_eq!(async_failure.status, Reported::Present(None));
    assert_eq!(async_failure.message, Reported::Present(None));
}

pub(super) fn assert_explicit_headers(call: &StreamCall<'_>, recorder: &mut Recorder) {
    let empty = SecretHeaders(BTreeMap::new());
    let mut explicit = Raised::plain("sent".to_owned());
    explicit.headers = Some(empty.clone());
    explicit.response_headers = Some(secret_headers(&[("retry-after", "9")]));
    let source = failing_source(ClientKind::AsyncAzureOpenAi, explicit);
    let failure = azure_failure(
        block_on(async_streaming(&source, call, recorder, runtime()))
            .err()
            .expect("explicit empty headers suppress the fallback"),
    );
    assert_eq!(failure.headers, Some(empty));
}

pub(super) fn assert_empty_stream_options(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    recorder: &mut Recorder,
) {
    let mut empty = CallFixture::default();
    empty
        .data
        .insert("stream_options".to_owned(), Json::Object(IndexMap::new()));
    let empty_call = empty.call();
    let empty_stream = block_on(streaming(source, sources, &empty_call, recorder, runtime()))
        .expect("empty stream options open");
    assert!(empty_stream.stream_options.is_none());
}
