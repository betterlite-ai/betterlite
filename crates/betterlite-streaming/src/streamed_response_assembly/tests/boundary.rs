use betterlite_error::error::ErrorClass;

use crate::streamed_response_assembly::AssemblyError;

use super::support::chunks;
use super::{Harness, failed};

const SOURCE_MESSAGE: &str = "Error building chunks for logging/streaming usage calculation";

pub(super) async fn empty_and_error_boundary() {
    missing_chunks_boundary().await;
    empty_input_boundary().await;
    base_metadata_boundary().await;
    text_error_boundary().await;
}

pub(super) async fn missing_chunks_boundary() {
    let (result, logs) = Harness::new(None).run().await;
    let failure = failed(result);
    assert!(matches!(&failure, AssemblyError::MissingChunks));
    assert_eq!(
        logs,
        vec![
            "litellm.main.py::stream_chunk_builder() - Exception occurred - chunks is required"
                .to_owned()
        ]
    );
    let ErrorClass::Api(details) = failure.classify() else {
        panic!("the builder error must classify as an API error");
    };
    assert_eq!(details.status, 500);
    assert_eq!(details.context.message, SOURCE_MESSAGE);
    assert_eq!(details.context.model.as_deref(), Some(""));
    assert_eq!(details.context.provider.as_deref(), Some(""));
}

pub(super) async fn empty_input_boundary() {
    let (result, logs) = Harness::new(Some(Vec::new())).run().await;
    assert!(matches!(result, Ok(None)));
    assert!(logs.is_empty());
}

pub(super) async fn base_metadata_boundary() {
    let cases = [
        (r#"[{"choices": []}]"#, "the first chunk has no object"),
        (
            r#"[{"object": "chat.completion.chunk", "model": "gpt-4o", "choices": []}]"#,
            "the first chunk has no created timestamp",
        ),
    ];
    for (source, message) in cases {
        let (result, logs) = Harness::new(Some(chunks(source))).run().await;
        assert!(matches!(failed(result), AssemblyError::Base(_)));
        assert_eq!(logs.len(), 1);
        assert!(logs[0].ends_with(&format!("Exception occurred - {message}")));
    }
}

pub(super) async fn text_error_boundary() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "text_completion", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "text": "Hi"}]},
        {"choices": []}
    ]"#,
    );
    let (result, logs) = Harness::new(Some(stream)).run().await;
    assert!(matches!(failed(result), AssemblyError::MissingLastChoice));
    assert_eq!(logs.len(), 1);
    assert!(logs[0].ends_with("Exception occurred - the last chunk has no first choice"));
}
