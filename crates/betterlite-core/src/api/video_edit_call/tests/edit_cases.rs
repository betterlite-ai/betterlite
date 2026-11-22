use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use futures_executor::block_on;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    INITIAL_CALL_TYPE, Reply, Rig, base_kwargs, call, done, locked, only_call, video,
};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::vector_store_http_write::service::ClientKind;
use crate::api::video_edit_call::{avideo_edit, video_edit};
use crate::api::video_edit_transport::calls::Handled;
use crate::api::video_edit_transport::config::{Headers, Params};
use crate::api::video_records::VideoObject;
use crate::api::video_selection::VideoConfigTag;

fn mock_string_answers_before_the_provider() {
    let rig = Rig::new(Reply::Done(video("handler")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(r#"{"id":"video_1","object":"video","status":"completed"}"#.to_owned()),
    );
    let request = call("not-a-video-id", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let answer = video_edit(&deps, &request).expect("the mock answers");
    assert_eq!(done(answer), video("video_1"));
    assert_eq!(rig.handler.calls().len(), 0, "no handler call");
    assert_eq!(rig.configs.asked().len(), 0, "no provider lookup");
    assert_eq!(
        locked(&rig).call_type,
        INITIAL_CALL_TYPE,
        "no logging update"
    );
}

fn mock_map_answers_before_the_provider() {
    let rig = Rig::new(Reply::Done(video("handler")), &[]);
    let mut mock = IndexMap::new();
    mock.insert("id".to_owned(), Json::Text("video_2".to_owned()));
    mock.insert("object".to_owned(), Json::Text("video".to_owned()));
    mock.insert("status".to_owned(), Json::Text("completed".to_owned()));
    let mut kwargs = base_kwargs();
    kwargs.insert("mock_response".to_owned(), Json::Object(mock));
    let request = call("not-a-video-id", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let answer = video_edit(&deps, &request).expect("the mock answers");
    assert_eq!(done(answer), video("video_2"));
    assert_eq!(rig.handler.calls().len(), 0, "no handler call");
    assert_eq!(rig.configs.asked().len(), 0, "no provider lookup");
}

fn provider_comes_from_the_decoded_video_id() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::Gemini]);
    let video_id = encode_video_id_with_provider("video_1", "gemini", None);
    let kwargs = base_kwargs();
    let request = call(&video_id, "make it blue", &kwargs);
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    assert_eq!(only_call(&rig).provider, "gemini");
    assert_eq!(rig.configs.asked(), vec![VideoConfigTag::Gemini]);
}

fn provider_defaults_to_openai() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    let recorded = only_call(&rig);
    assert_eq!(recorded.provider, "openai");
    assert_eq!(recorded.client, None, "no client keyword, no client");
}

fn an_empty_decoded_provider_defaults_to_openai() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    // base64 of "litellm:custom_llm_provider:;model_id:;video_id:video_1"
    let video_id =
        "video_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOjttb2RlbF9pZDo7dmlkZW9faWQ6dmlkZW9fMQ==";
    let decoded = decode_video_id_with_provider(video_id);
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some(""));
    let kwargs = base_kwargs();
    let request = call(video_id, "make it blue", &kwargs);
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    assert_eq!(only_call(&rig).provider, "openai");
}

fn explicit_provider_skips_the_decode() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::Azure]);
    let video_id = encode_video_id_with_provider("video_1", "gemini", None);
    let kwargs = base_kwargs();
    let mut request = call(&video_id, "make it blue", &kwargs);
    request.custom_llm_provider = Some("azure");
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    assert_eq!(only_call(&rig).provider, "azure");
    assert_eq!(rig.configs.asked(), vec![VideoConfigTag::Azure]);
}

fn logging_update_carries_the_request_params() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert("user".to_owned(), Json::Text("u-1".to_owned()));
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    let state = locked(&rig);
    assert_eq!(state.model, "", "model is the empty string");
    assert_eq!(state.call_type, "video_edit");
    assert_eq!(state.user.as_deref(), Some("u-1"));
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        state
            .optional_params
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec!["video_id".to_owned(), "prompt".to_owned()]
    );
    assert_eq!(
        state.litellm_params.get("video_id"),
        Some(&Json::Text("video_1".to_owned()))
    );
}

fn the_logging_params_start_with_the_call_id() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text("call-9".to_owned()),
    );
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    let state = locked(&rig);
    assert_eq!(
        state
            .litellm_params
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec![
            "litellm_call_id".to_owned(),
            "video_id".to_owned(),
            "prompt".to_owned()
        ]
    );
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-9".to_owned()))
    );
    assert_eq!(
        state.litellm_params.get("prompt"),
        Some(&Json::Text("make it blue".to_owned()))
    );
    let recorded = only_call(&rig);
    assert_eq!(
        recorded.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-9".to_owned())),
        "the call id is read, not removed"
    );
}

fn handler_receives_the_load_bearing_arguments() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let media = SuppliedMedia::Bytes(vec![1_u8, 2, 3]);
    let headers = Headers::from([("x-extra".to_owned(), "yes".to_owned())]);
    let body = Params::from([("seed".to_owned(), Json::Number(Number::from(7_u8)))]);
    let query = Params::from([("ignored".to_owned(), Json::Bool(true))]);
    let kwargs = base_kwargs();
    let mut request = call("video_1", "make it blue", &kwargs);
    request.custom_llm_provider = Some("openai");
    request.video = Some(&media);
    request.extra_headers = Some(&headers);
    request.extra_query = Some(&query);
    request.extra_body = Some(&body);
    let deps = rig.deps(Some(ClientKind::Sync));
    let _answer = video_edit(&deps, &request).expect("the call runs");
    let recorded = only_call(&rig);
    assert_eq!(recorded.prompt, "make it blue");
    assert_eq!(recorded.video_id, "video_1");
    assert_eq!(recorded.video_file, Some(media.clone()));
    assert_eq!(recorded.extra_headers, Some(headers.clone()));
    assert_eq!(recorded.extra_body, Some(body.clone()));
    assert_eq!(recorded.client, Some((ClientKind::Sync, "supplied")));
    assert_eq!(recorded.timeout, Some(Duration::from_secs(600)));
    assert!(!recorded.api_key_present, "api_key is never passed");
    assert!(!recorded.is_async);
    assert!(
        !recorded.litellm_params.contains_key("litellm_logging_obj"),
        "the logging object leaves the params"
    );
}

fn zero_timeout_falls_to_the_request_timeout() {
    let mut rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    rig.request_timeout = Duration::from_secs(1_234);
    let kwargs = base_kwargs();
    let mut request = call("video_1", "make it blue", &kwargs);
    request.timeout = Some(Duration::ZERO);
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    assert_eq!(only_call(&rig).timeout, Some(Duration::from_secs(1_234)));
}

fn a_string_async_call_stays_sync() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert("async_call".to_owned(), Json::Text("true".to_owned()));
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let answer = video_edit(&deps, &request).expect("the call runs");
    assert!(!only_call(&rig).is_async, "a string is not the boolean");
    assert_eq!(done(answer), video("video_1"));
}

fn the_number_one_async_call_stays_sync() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert("async_call".to_owned(), Json::Number(Number::from(1_u8)));
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let answer = video_edit(&deps, &request).expect("the call runs");
    assert!(!only_call(&rig).is_async, "a number is not the boolean");
    assert_eq!(done(answer), video("video_1"));
}

fn an_async_call_answers_with_the_pending_future() {
    let rig = Rig::new(Reply::Pending(video("video_3")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert("async_call".to_owned(), Json::Bool(true));
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let answer = video_edit(&deps, &request).expect("the call runs");
    let Handled::Pending(pending) = answer else {
        panic!("async_call asks for the future");
    };
    let recorded = only_call(&rig);
    assert!(recorded.is_async);
    assert!(
        !recorded.litellm_params.contains_key("async_call"),
        "async_call leaves the params"
    );
    assert_eq!(
        block_on(pending).expect("the pending answer"),
        video("video_3")
    );
}

fn the_async_entry_awaits_the_pending_future() {
    let rig = Rig::new(Reply::Pending(video("video_3")), &[VideoConfigTag::Gemini]);
    let video_id = encode_video_id_with_provider("video_1", "gemini", None);
    let kwargs = base_kwargs();
    let request = call(&video_id, "make it blue", &kwargs);
    let deps = rig.deps(None);
    let object = block_on(avideo_edit(&deps, &request)).expect("the awaited answer");
    assert_eq!(object, video("video_3"));
    let recorded = only_call(&rig);
    assert!(recorded.is_async, "avideo_edit sets async_call to true");
    assert_eq!(
        recorded.provider, "gemini",
        "the provider comes from the sync decode, and the wrapper adds no inference"
    );
}

fn the_async_entry_returns_a_ready_value_as_is() {
    let rig = Rig::new(Reply::Done(video("video_5")), &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let object = block_on(avideo_edit(&deps, &request)).expect("the ready answer");
    assert_eq!(object, video("video_5"));
    assert!(
        only_call(&rig).is_async,
        "the transport sees the async flag"
    );
}

fn the_async_entry_returns_the_mock_as_is() {
    let rig = Rig::new(Reply::Done(video("handler")), &[]);
    let mut kwargs = base_kwargs();
    kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(r#"{"id":"video_4","object":"video","status":"queued"}"#.to_owned()),
    );
    let request = call("not-a-video-id", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let object = block_on(avideo_edit(&deps, &request)).expect("the mock answers");
    assert_eq!(
        object,
        VideoObject::new("video_4".to_owned(), "queued".to_owned())
    );
    assert_eq!(rig.handler.calls().len(), 0, "no handler call");
}

fn a_null_mock_response_is_not_a_mock() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert("mock_response".to_owned(), Json::Null);
    let request = call("video_1", "make it blue", &kwargs);
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    assert_eq!(
        only_call(&rig).litellm_params.get("mock_response"),
        Some(&Json::Null),
        "the mock key is read, not removed"
    );
}

fn an_explicit_timeout_reaches_the_handler() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let mut request = call("video_1", "make it blue", &kwargs);
    request.timeout = Some(Duration::from_secs(42));
    let deps = rig.deps(None);
    let _answer = video_edit(&deps, &request).expect("the call runs");
    assert_eq!(only_call(&rig).timeout, Some(Duration::from_secs(42)));
}

#[rstest]
#[case::mock_string_answers_before_the_provider(mock_string_answers_before_the_provider)]
#[case::mock_map_answers_before_the_provider(mock_map_answers_before_the_provider)]
#[case::provider_comes_from_the_decoded_video_id(provider_comes_from_the_decoded_video_id)]
#[case::provider_defaults_to_openai(provider_defaults_to_openai)]
#[case::an_empty_decoded_provider_defaults_to_openai(an_empty_decoded_provider_defaults_to_openai)]
#[case::explicit_provider_skips_the_decode(explicit_provider_skips_the_decode)]
#[case::logging_update_carries_the_request_params(logging_update_carries_the_request_params)]
#[case::the_logging_params_start_with_the_call_id(the_logging_params_start_with_the_call_id)]
#[case::handler_receives_the_load_bearing_arguments(handler_receives_the_load_bearing_arguments)]
#[case::zero_timeout_falls_to_the_request_timeout(zero_timeout_falls_to_the_request_timeout)]
#[case::an_explicit_timeout_reaches_the_handler(an_explicit_timeout_reaches_the_handler)]
#[case::a_string_async_call_stays_sync(a_string_async_call_stays_sync)]
#[case::the_number_one_async_call_stays_sync(the_number_one_async_call_stays_sync)]
#[case::an_async_call_answers_with_the_pending_future(
    an_async_call_answers_with_the_pending_future
)]
#[case::the_async_entry_awaits_the_pending_future(the_async_entry_awaits_the_pending_future)]
#[case::the_async_entry_returns_a_ready_value_as_is(the_async_entry_returns_a_ready_value_as_is)]
#[case::the_async_entry_returns_the_mock_as_is(the_async_entry_returns_the_mock_as_is)]
#[case::a_null_mock_response_is_not_a_mock(a_null_mock_response_is_not_a_mock)]
fn edit_call_behavior(#[case] check: fn()) {
    check();
}
