use betterlite_models::decisions::Json;
use futures_executor::block_on;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    INITIAL_CALL_TYPE, Reply, Rig, base_kwargs, call, connection, internal, locked, video,
};
use crate::api::video_extension_call::error::VideoExtensionCallError;
use crate::api::video_extension_call::{avideo_extension, video_extension};
use crate::api::video_selection::VideoConfigTag;

fn a_missing_logging_object_fails() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let kwargs = IndexMap::new();
    let request = call("video_1", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("the logging object is required");
    };
    assert!(matches!(
        failure.error,
        VideoExtensionCallError::MissingLoggingObject
    ));
    let context = connection(&failure.class);
    assert!(
        context.message.contains("litellm_logging_obj"),
        "the message names the keyword: {}",
        context.message
    );
    assert_eq!(
        context.provider.as_deref(),
        Some(""),
        "no provider is known yet"
    );
    assert_eq!(
        context.model.as_deref(),
        Some(""),
        "model is the empty string"
    );
    assert_eq!(rig.handler.calls().len(), 0, "nothing reaches the handler");
    assert_eq!(rig.configs.asked().len(), 0, "no provider lookup");
    assert_eq!(
        locked(&rig).call_type,
        INITIAL_CALL_TYPE,
        "no logging update"
    );
}

fn a_null_logging_object_fails() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = IndexMap::new();
    kwargs.insert("litellm_logging_obj".to_owned(), Json::Null);
    let request = call("video_1", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("a null logging object is required too");
    };
    assert!(matches!(
        failure.error,
        VideoExtensionCallError::MissingLoggingObject
    ));
    let context = connection(&failure.class);
    assert!(context.message.contains("litellm_logging_obj"));
    assert_eq!(rig.handler.calls().len(), 0);
}

fn a_broken_mock_string_fails() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert(
        "mock_response".to_owned(),
        Json::Text("{not json".to_owned()),
    );
    let request = call("not-a-video-id", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("an unparsable mock is an error");
    };
    assert!(matches!(failure.error, VideoExtensionCallError::Mock(_)));
    let context = connection(&failure.class);
    assert!(
        context.message.contains("not valid JSON"),
        "the message names the decode: {}",
        context.message
    );
    assert_eq!(
        rig.configs.asked().len(),
        0,
        "the mock fails before the provider"
    );
    assert_eq!(rig.handler.calls().len(), 0);
}

fn a_mock_that_is_not_a_video_object_fails() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut mock = IndexMap::new();
    mock.insert("id".to_owned(), Json::Text("video_2".to_owned()));
    let mut kwargs = base_kwargs();
    kwargs.insert("mock_response".to_owned(), Json::Object(mock));
    let request = call("not-a-video-id", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("a map without the required fields is an error");
    };
    assert!(matches!(failure.error, VideoExtensionCallError::Mock(_)));
    let context = connection(&failure.class);
    assert!(
        context.message.contains("does not match a video object"),
        "the message names the shape: {}",
        context.message
    );
    assert_eq!(rig.handler.calls().len(), 0);
}

fn an_unknown_provider_fails() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let mut request = call("video_1", "extend it", "8", &kwargs);
    request.custom_llm_provider = Some("bogus");
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("bogus is not a provider");
    };
    assert!(matches!(
        failure.error,
        VideoExtensionCallError::UnknownProvider(_)
    ));
    let context = connection(&failure.class);
    assert!(
        context
            .message
            .contains("is not a provider betterlite knows"),
        "the message names the conversion: {}",
        context.message
    );
    assert!(context.message.contains("bogus"));
    assert_eq!(context.provider.as_deref(), Some("bogus"));
    assert_eq!(
        context.model.as_deref(),
        Some(""),
        "model is the empty string"
    );
    assert_eq!(rig.configs.asked().len(), 0);
}

fn an_unsupported_provider_keeps_litellms_message() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let mut request = call("video_1", "extend it", "8", &kwargs);
    request.custom_llm_provider = Some("mistral");
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("mistral has no video config");
    };
    assert!(matches!(
        failure.error,
        VideoExtensionCallError::ProviderConfigMissing { .. }
    ));
    let context = connection(&failure.class);
    assert_eq!(
        context.message,
        "APIConnectionError: MistralException - video extension is not supported for mistral"
    );
    assert_eq!(context.provider.as_deref(), Some("mistral"));
    assert_eq!(
        context.model.as_deref(),
        Some(""),
        "model is the empty string"
    );
    assert_eq!(
        rig.configs.asked().len(),
        0,
        "the selector answers none before the lookup"
    );
    assert_eq!(rig.handler.calls().len(), 0);
}

fn edenai_reports_the_selection_message() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let mut request = call("video_1", "extend it", "8", &kwargs);
    request.custom_llm_provider = Some("edenai");
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("edenai has no video config yet");
    };
    assert!(matches!(
        failure.error,
        VideoExtensionCallError::Selection(_)
    ));
    let context = connection(&failure.class);
    assert!(
        context
            .message
            .contains("no video config is installed for 'edenai'"),
        "the selector's message survives: {}",
        context.message
    );
    assert_eq!(context.provider.as_deref(), Some("edenai"));
    assert_eq!(
        context.model.as_deref(),
        Some(""),
        "model is the empty string"
    );
    assert_eq!(rig.handler.calls().len(), 0);
}

fn a_handler_provider_error_keeps_its_status() {
    let rig = Rig::new(
        Reply::Failed {
            status_code: 500,
            message: "boom".to_owned(),
        },
        &[VideoConfigTag::OpenAi],
    );
    let kwargs = base_kwargs();
    let request = call("video_1", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("the handler fails");
    };
    assert!(matches!(failure.error, VideoExtensionCallError::Handler(_)));
    let context = internal(&failure.class);
    assert_eq!(
        context.message,
        "InternalServerError: OpenAIException - boom"
    );
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(
        context.model.as_deref(),
        Some(""),
        "model is the empty string"
    );
    assert_eq!(rig.handler.calls().len(), 1);
}

fn a_handler_field_type_error_is_a_value_error() {
    let rig = Rig::new(Reply::Broken, &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let request = call("video_1", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("the handler fails");
    };
    assert!(matches!(failure.error, VideoExtensionCallError::Handler(_)));
    let context = connection(&failure.class);
    assert_eq!(
        context.message,
        "APIConnectionError: OpenAIException - litellm_params.ssl_verify must be a boolean"
    );
    assert_eq!(rig.handler.calls().len(), 1);
}

fn the_sync_path_refuses_a_pending_future() {
    let rig = Rig::new(Reply::Pending(video("video_3")), &[VideoConfigTag::OpenAi]);
    let kwargs = base_kwargs();
    let request = call("video_1", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("a sync handler cannot answer with a future");
    };
    assert!(matches!(
        failure.error,
        VideoExtensionCallError::SyncPending
    ));
    let context = connection(&failure.class);
    assert_eq!(
        context.message,
        "APIConnectionError: OpenAIException - the video extension handler answered with a pending future on its sync path"
    );
    assert_eq!(rig.handler.calls().len(), 1);
}

fn a_non_text_user_is_refused() {
    let rig = Rig::new(Reply::Done(video("video_1")), &[VideoConfigTag::OpenAi]);
    let mut kwargs = base_kwargs();
    kwargs.insert("user".to_owned(), Json::Bool(true));
    let request = call("video_1", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = video_extension(&deps, &request) else {
        panic!("a boolean user is not a user");
    };
    assert!(matches!(failure.error, VideoExtensionCallError::Logging(_)));
    let context = connection(&failure.class);
    assert_eq!(
        context.message,
        "APIConnectionError: OpenAIException - the user value must be a string"
    );
    assert_eq!(rig.handler.calls().len(), 0);
}

fn the_async_entry_maps_a_pending_error_with_the_caller_provider() {
    let rig = Rig::new(
        Reply::Failed {
            status_code: 500,
            message: "boom".to_owned(),
        },
        &[VideoConfigTag::OpenAi],
    );
    let kwargs = base_kwargs();
    let request = call("video_1", "extend it", "8", &kwargs);
    let deps = rig.deps(None);
    let Err(failure) = block_on(avideo_extension(&deps, &request)) else {
        panic!("the pending call fails");
    };
    let context = internal(&failure.class);
    assert_eq!(
        context.message, "Exception - boom",
        "the caller's empty provider reaches exception_type"
    );
    assert_eq!(
        context.provider.as_deref(),
        Some(""),
        "the wrapper infers no provider"
    );
    assert_eq!(
        context.model.as_deref(),
        Some(""),
        "model is the empty string"
    );
    assert_eq!(rig.handler.calls().len(), 1);
}

#[rstest]
#[case::a_missing_logging_object_fails(a_missing_logging_object_fails)]
#[case::a_null_logging_object_fails(a_null_logging_object_fails)]
#[case::a_broken_mock_string_fails(a_broken_mock_string_fails)]
#[case::a_mock_that_is_not_a_video_object_fails(a_mock_that_is_not_a_video_object_fails)]
#[case::an_unknown_provider_fails(an_unknown_provider_fails)]
#[case::an_unsupported_provider_keeps_litellms_message(
    an_unsupported_provider_keeps_litellms_message
)]
#[case::edenai_reports_the_selection_message(edenai_reports_the_selection_message)]
#[case::a_handler_provider_error_keeps_its_status(a_handler_provider_error_keeps_its_status)]
#[case::a_handler_field_type_error_is_a_value_error(a_handler_field_type_error_is_a_value_error)]
#[case::the_sync_path_refuses_a_pending_future(the_sync_path_refuses_a_pending_future)]
#[case::a_non_text_user_is_refused(a_non_text_user_is_refused)]
#[case::the_async_entry_maps_a_pending_error_with_the_caller_provider(
    the_async_entry_maps_a_pending_error_with_the_caller_provider
)]
fn call_error_behavior(#[case] check: fn()) {
    check();
}
