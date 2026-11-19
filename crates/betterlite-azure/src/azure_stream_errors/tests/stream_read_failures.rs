use futures_util::StreamExt as _;

use crate::azure_stream_errors::error::{Raised, Reported};
use crate::azure_stream_errors::records::StreamCall;
use crate::azure_stream_errors::{async_streaming, streaming};
use crate::endpoints::projection::ClientKind;

use super::fixture::{
    CallFixture, Recorder, ScriptedSource, ScriptedSources, azure_failure, block_on,
    failing_source, runtime, scripted_source,
};

pub(super) fn assert_stream_failures(
    sources: &ScriptedSources,
    call: &StreamCall<'_>,
    recorder: &mut Recorder,
) {
    let read_source = scripted_source(
        ClientKind::AzureOpenAi,
        &[],
        vec![Err("the connection dropped".to_owned())],
    );
    let mut stream = block_on(streaming(&read_source, sources, call, recorder, runtime()))
        .expect("the stream opens before its bytes fail");
    let read_error = block_on(async { stream.chunks.next().await })
        .expect("one failing item")
        .err()
        .expect("the read failure is reported");
    let read_failure = azure_failure(read_error);
    assert_eq!(read_failure.class, "APIError");
    assert_eq!(read_failure.status, Reported::Absent);
    assert_eq!(read_failure.message, Reported::Absent);

    let send_source = failing_source(
        ClientKind::AsyncAzureOpenAi,
        Raised::plain("socket closed".to_owned()),
    );
    let send_failure = azure_failure(
        block_on(async_streaming(&send_source, call, recorder, runtime()))
            .err()
            .expect("async wraps the send failure"),
    );
    assert_eq!(send_failure.class, "AzureOpenAIError");
    assert_eq!(send_failure.status, Reported::Present(Some(500)));
    assert_eq!(
        send_failure.message,
        Reported::Present(Some("socket closed".to_owned()))
    );
    assert_eq!(send_failure.text, "socket closed");
}

pub(super) fn assert_absent_stream_options(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    recorder: &mut Recorder,
) {
    let absent = CallFixture::default();
    let absent_call = absent.call();
    let absent_stream = block_on(streaming(
        source,
        sources,
        &absent_call,
        recorder,
        runtime(),
    ))
    .expect("absent stream options open");
    assert_eq!(absent_stream.stream_options, None);
}
