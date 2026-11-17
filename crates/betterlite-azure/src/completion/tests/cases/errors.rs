use crate::completion::error::{ASYNC_CLIENT, MISSING_MODEL_OR_MESSAGES, SYNC_CLIENT};
use crate::completion::tests::fixture::Answer;
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::ClientKind;

use super::support::{
    Scenario, async_buffered, async_stream, buffered, sync_buffered, sync_stream, text,
};

pub(super) fn completion_errors() {
    let status = Answer::Status {
        status: 429,
        message: "slow down".to_owned(),
        headers: vec![("retry-after".to_owned(), "7".to_owned())],
    };
    let run = Scenario::new(ClientKind::AzureOpenAi, status, "{}").execute();
    assert_eq!(run.failure().status, Some(429));
    assert_eq!(run.failure().message, "slow down");
    let headers = run.failure().headers.as_ref().unwrap();
    assert_eq!(headers.0.get("retry-after").map(String::as_str), Some("7"));
    assert_eq!(run.hooks.pre.len(), 1);
    assert!(run.hooks.post.is_empty());
    let other = Answer::Other("boom".to_owned());
    let boom = Scenario::new(ClientKind::AzureOpenAi, other, "{}").execute();
    assert_eq!(boom.failure().status, Some(500));
    assert_eq!(boom.failure().message, "boom");
}

fn wrong_client_kinds() {
    let sync_call = Scenario::new(ClientKind::OpenAi, buffered(), "{}").execute();
    assert_eq!(sync_call.failure().status, Some(500));
    assert_eq!(sync_call.failure().message, SYNC_CLIENT);
    assert_eq!(sync_call.source.gets().len(), 1);
    assert!(sync_call.source.creates().is_empty());
    assert_eq!(sync_call.hooks.pre.len(), 1);
    let mut async_call = sync_buffered("{}");
    async_call.acompletion = true;
    assert_eq!(async_call.execute().failure().message, ASYNC_CLIENT);
    let mut async_streaming = sync_stream(r#"{"stream":true}"#);
    async_streaming.acompletion = true;
    assert_eq!(async_streaming.execute().failure().message, ASYNC_CLIENT);
    let mut sync_streaming = async_stream(r#"{"stream":true}"#);
    sync_streaming.acompletion = false;
    assert_eq!(sync_streaming.execute().failure().message, SYNC_CLIENT);
}

fn failing_callbacks() {
    let scenarios = [
        sync_buffered("{}"),
        async_buffered("{}"),
        async_stream(r#"{"stream":true}"#),
        sync_stream(r#"{"stream":true}"#),
    ];
    for mut scenario in scenarios {
        scenario.fail_pre = true;
        let run = scenario.execute();
        assert_eq!(run.failure().status, Some(500));
        assert_eq!(run.failure().message, "hook failed");
        assert!(run.source.creates().is_empty());
    }
}

pub(super) fn azure_text_client_errors() {
    wrong_client_kinds();
    let mut missing = sync_buffered("{}");
    missing.source.fail_get = true;
    let run = missing.execute();
    assert_eq!(run.failure().status, Some(500));
    let expected = EndpointError::MissingApiVersion.to_string();
    assert_eq!(run.failure().message, expected);
    assert!(run.source.creates().is_empty());
    failing_callbacks();
}

pub(super) fn missing_model_messages_422() {
    let mut no_model = sync_buffered("{}");
    no_model.parts.model = None;
    let run = no_model.execute();
    assert_eq!(run.failure().status, Some(422));
    assert_eq!(run.failure().message, MISSING_MODEL_OR_MESSAGES);
    assert!(run.hooks.pre.is_empty());
    assert!(run.source.gets().is_empty());
    let mut no_messages = sync_buffered("{}");
    no_messages.messages = None;
    assert_eq!(no_messages.execute().failure().status, Some(422));
    let mut empty = sync_buffered("{}");
    empty.parts.model = Some(String::new());
    empty.messages = Some(Vec::new());
    let proceeded = empty.execute();
    assert!(proceeded.reply.is_some());
    assert_eq!(text(&proceeded.create("model")), "");
}
