#![cfg(test)]

use rstest::rstest;

use super::support::{ENDPOINT, Fixture, MODEL, STRIPPED_MODEL};
use crate::gemma_call::error::GemmaPlanError;
use crate::gemma_call::plan::{CallerStyle, Invocation, SuppliedClientKind, for_call};

#[test]
fn required_base() {
    let fixture = Fixture::new();
    let missing = fixture.call(MODEL, None, false);
    let error = for_call(&missing, None).unwrap_err();
    assert!(matches!(error, GemmaPlanError::MissingApiBase));
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("api_base"));
    let empty = fixture.call(MODEL, Some(""), false);
    let empty_error = for_call(&empty, None).unwrap_err();
    assert!(matches!(empty_error, GemmaPlanError::MissingApiBase));
}

#[rstest]
#[case::already_predict(
    "https://ep.example/v1/projects/p/locations/l/endpoints/e:predict",
    false,
    "https://ep.example/v1/projects/p/locations/l/endpoints/e:predict"
)]
#[case::already_predict_stream(
    "https://ep.example/v1/projects/p/locations/l/endpoints/e:predict",
    true,
    "https://ep.example/v1/projects/p/locations/l/endpoints/e:predict"
)]
#[case::host_only("https://ep.example", false, "https://ep.example")]
#[case::host_only_stream("https://ep.example", true, "https://ep.example?alt=sse")]
#[case::trailing_slash("https://ep.example/", false, "https://ep.example/")]
#[case::custom_path(
    "https://ep.example/custom",
    false,
    "https://ep.example/custom:predict"
)]
#[case::custom_path_stream(
    "https://ep.example/custom",
    true,
    "https://ep.example/custom:predict?alt=sse"
)]
#[case::version_root("https://ep.example/v1", false, "https://ep.example/v1:predict")]
#[case::deployment_path(
    "https://ep.example/v1/projects/p/locations/l/endpoints/e",
    false,
    "https://ep.example/v1/projects/p/locations/l/endpoints/e/projects/p/locations/l/endpoints/e"
)]
fn predict_suffix_preserved(#[case] api_base: &str, #[case] stream: bool, #[case] expected: &str) {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(api_base), stream);
    let planned = for_call(&call, None).unwrap();
    assert_eq!(planned.url, expected);
    assert_eq!(planned.stream, stream);
}

#[rstest]
#[case::async_handler(
    Invocation::Async,
    Some(SuppliedClientKind::AsyncHandler),
    Some(CallerStyle::Handler)
)]
#[case::async_raw(
    Invocation::Async,
    Some(SuppliedClientKind::AsyncRaw),
    Some(CallerStyle::Raw)
)]
#[case::async_with_sync_handler(Invocation::Async, Some(SuppliedClientKind::SyncHandler), None)]
#[case::async_with_sync_raw(Invocation::Async, Some(SuppliedClientKind::SyncRaw), None)]
#[case::sync_handler(
    Invocation::Sync,
    Some(SuppliedClientKind::SyncHandler),
    Some(CallerStyle::Handler)
)]
#[case::sync_raw(
    Invocation::Sync,
    Some(SuppliedClientKind::SyncRaw),
    Some(CallerStyle::Raw)
)]
#[case::sync_with_async_handler(Invocation::Sync, Some(SuppliedClientKind::AsyncHandler), None)]
#[case::sync_with_async_raw(Invocation::Sync, Some(SuppliedClientKind::AsyncRaw), None)]
#[case::no_client(Invocation::Async, None, None)]
fn matching_client_kind(
    #[case] invocation: Invocation,
    #[case] supplied: Option<SuppliedClientKind>,
    #[case] expected: Option<CallerStyle>,
) {
    let fixture = Fixture::new();
    let mut call = fixture.call(MODEL, Some(ENDPOINT), false);
    call.invocation = invocation;
    let planned = for_call(&call, supplied).unwrap();
    assert_eq!(planned.caller, expected);
}

#[test]
fn false_stream_default() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    assert!(!call.stream);
    let planned = for_call(&call, None).unwrap();
    assert!(!planned.stream);
    assert!(!planned.url.contains("alt=sse"));
    // the normalized field decides, not a stream the raw options still carry
    let raw: betterlite_prompts::arguments::json::Object =
        serde_json::from_str(r#"{"stream":true}"#).unwrap();
    let mut carried = fixture.call(MODEL, Some(ENDPOINT), false);
    carried.optional_params = &raw;
    assert!(!for_call(&carried, None).unwrap().stream);
}

#[test]
fn restored_model() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let planned = for_call(&call, None).unwrap();
    assert_eq!(planned.model, STRIPPED_MODEL);
    let plain = fixture.call(STRIPPED_MODEL, Some(ENDPOINT), false);
    assert_eq!(for_call(&plain, None).unwrap().model, STRIPPED_MODEL);
}
