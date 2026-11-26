use betterlite_error::error::ErrorClass;

use super::support::{
    Bench, FakeTransport, Outcome, VIDEO_ID, handled, handles, lookups, request, text,
};
use crate::api::video_generation_call::CALL_TYPE;
use crate::api::video_generation_call::error::{VideoGenerationCallError, failure};
use crate::api::videos_http_transport::client::ClientKind;

#[tokio::test]
async fn the_async_entry_infers_the_provider_from_the_default_model() {
    let mut bench = Bench::new(Outcome::Answers);
    let object = bench
        .async_run(request(), None)
        .await
        .expect("the async call answers");
    assert_eq!(object.id, VIDEO_ID);
    let inner = handled(&bench.journal);
    assert_eq!(inner.model, "sora-2");
    assert_eq!(inner.custom_llm_provider, "openai");
    assert!(inner.is_async);
    assert_eq!(inner.client, None);
    assert_eq!(
        lookups(&bench.journal),
        vec!["OpenAIVideoConfig".to_owned()]
    );
    assert_eq!(bench.fixture.state.call_type, CALL_TYPE);
}

#[tokio::test]
async fn the_async_inference_reads_an_empty_model_as_the_default() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.model = Some(String::new());
    bench
        .async_run(call, None)
        .await
        .expect("the async call answers");
    let inner = handled(&bench.journal);
    assert_eq!(inner.model, "sora-2");
    assert_eq!(inner.custom_llm_provider, "openai");
    assert!(inner.is_async);
}

#[tokio::test]
async fn the_inference_ignores_a_loose_api_base_but_the_handler_still_gets_it() {
    let api_base = "https://api.groq.com/openai/v1";
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs.insert("api_base".to_owned(), text(api_base));
    bench
        .async_run(call, None)
        .await
        .expect("the async call answers");
    let inner = handled(&bench.journal);
    assert_eq!(inner.custom_llm_provider, "openai");
    assert_eq!(inner.litellm_params.get("api_base"), Some(&text(api_base)));
}

#[tokio::test]
async fn a_supplied_client_reaches_the_async_handler() {
    let mut bench = Bench::new(Outcome::Answers);
    let transport = FakeTransport;
    bench
        .async_run(request(), Some(&transport))
        .await
        .expect("the async call answers");
    let inner = handled(&bench.journal);
    assert_eq!(inner.client, Some(ClientKind::Asynchronous));
    assert!(inner.is_async);
}

#[tokio::test]
async fn a_sync_path_failure_passes_through_the_mapping_it_already_got() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.custom_llm_provider = Some("anthropic".to_owned());
    let raised = bench
        .async_run(call, None)
        .await
        .expect_err("the provider is unsupported");
    assert_eq!(
        raised.error.to_string(),
        "video generation is not supported for anthropic"
    );
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!("the sync mapping passes through, got {:?}", raised.class);
    };
    assert_eq!(details.model.as_deref(), Some("sora-2"));
    assert_eq!(details.provider.as_deref(), Some("anthropic"));
    assert!(handles(&bench.journal).is_empty());
}

#[tokio::test]
async fn a_pending_handler_failure_is_mapped_with_the_raw_model() {
    let mut bench = Bench::new(Outcome::FieldType);
    let raised = bench
        .async_run(request(), None)
        .await
        .expect_err("the handler fails");
    let expected = failure(
        VideoGenerationCallError::FieldType {
            field: "size",
            expected: "a string",
        },
        None,
        Some("openai"),
        &bench.caller.redactor,
    );
    assert_eq!(raised.class, expected.class);
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!("the pending failure answers 400, got {:?}", raised.class);
    };
    assert_eq!(details.model, None);
    let resolved = failure(
        VideoGenerationCallError::FieldType {
            field: "size",
            expected: "a string",
        },
        Some("sora-2"),
        Some("openai"),
        &bench.caller.redactor,
    );
    assert_ne!(
        raised.class, resolved.class,
        "the async path maps with the raw model, not the resolved one"
    );
}
