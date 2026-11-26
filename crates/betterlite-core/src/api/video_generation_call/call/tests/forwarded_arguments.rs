use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{
    Bench, FakeTransport, Outcome, VIDEO_ID, handled, headers, params, request, text,
};
use crate::api::video_generation_call::VideoGenerationRequest;
use crate::api::videos_http_transport::client::ClientKind;

// a call whose named and loose options disagree, so every precedence step shows in what the handler gets
fn merged_sorting_call() -> VideoGenerationRequest {
    let mut call = request();
    call.timeout = Some(5.0_f64);
    call.extra_headers = Some(headers(&[("x-extra", "1")]));
    call.extra_query = Some(params(&[("after", text("cursor"))]));
    call.extra_body = Some(params(&[("size", text("720x1280"))]));
    call.kwargs.insert(
        "extra_body".to_owned(),
        Json::Object(IndexMap::from([
            ("size".to_owned(), text("480x854")),
            ("seed".to_owned(), text("7")),
        ])),
    );
    call.kwargs.insert("size".to_owned(), text("1080x1920"));
    call
}

#[test]
fn the_handler_gets_the_original_extra_body_the_client_and_the_timeout() {
    let mut bench = Bench::new(Outcome::Answers);
    let transport = FakeTransport;
    let object = bench
        .run_with(merged_sorting_call(), Some(&transport))
        .expect("the call answers");
    assert_eq!(object.id, VIDEO_ID);
    let inner = handled(&bench.journal);
    assert_eq!(inner.timeout, Duration::from_secs(5));
    assert_eq!(inner.extra_headers, Some(headers(&[("x-extra", "1")])));
    assert_eq!(
        inner.extra_body,
        Some(params(&[("size", text("720x1280"))]))
    );
    assert_eq!(inner.client, Some(ClientKind::Sync));
    assert!(!inner.is_async);
    assert!(!inner.optional_params.contains_key("extra_query"));
    assert!(!inner.litellm_params.contains_key("extra_query"));
    assert_eq!(
        inner.litellm_params.get("extra_body"),
        Some(&Json::Object(IndexMap::from([
            ("size".to_owned(), text("480x854")),
            ("seed".to_owned(), text("7")),
        ])))
    );
    assert_eq!(inner.litellm_params.get("size"), Some(&text("1080x1920")));
}

#[test]
fn the_provider_mapping_sees_the_merged_options() {
    let mut bench = Bench::new(Outcome::Answers);
    bench
        .run_with(merged_sorting_call(), None)
        .expect("the call answers");
    let mapped = bench.caller.providers.config.calls();
    assert_eq!(mapped.len(), 1_usize);
    assert_eq!(
        mapped[0].options.extra_headers,
        Some(IndexMap::from([("x-extra".to_owned(), "1".to_owned())]))
    );
    assert_eq!(mapped[0].options.size.as_deref(), Some("480x854"));
    assert_eq!(
        mapped[0].options.extra_body,
        Some(IndexMap::from([
            ("size".to_owned(), "480x854".to_owned()),
            ("seed".to_owned(), "7".to_owned()),
        ]))
    );
}

#[test]
fn the_logging_object_and_the_async_flag_are_popped_before_the_params_are_built() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs.insert(
        "litellm_logging_obj".to_owned(),
        Json::Object(IndexMap::new()),
    );
    call.kwargs
        .insert("async_call".to_owned(), Json::Bool(false));
    bench.run(call).expect("the call answers");
    let inner = handled(&bench.journal);
    assert!(!inner.litellm_params.contains_key("litellm_logging_obj"));
    assert!(!inner.litellm_params.contains_key("async_call"));
    assert!(!inner.is_async);
}

#[test]
fn a_true_async_flag_leaves_the_pending_handler_with_an_async_client() {
    let mut bench = Bench::new(Outcome::Answers);
    let transport = FakeTransport;
    let mut call = request();
    call.kwargs
        .insert("async_call".to_owned(), Json::Bool(true));
    assert!(bench.leaves_pending(call, Some(&transport)));
    let inner = handled(&bench.journal);
    assert!(inner.is_async);
    assert_eq!(inner.client, Some(ClientKind::Asynchronous));
}

#[test]
fn only_the_bool_true_async_flag_counts() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs.insert("async_call".to_owned(), text("true"));
    assert!(!bench.leaves_pending(call, None));
    assert!(!handled(&bench.journal).is_async);
}
