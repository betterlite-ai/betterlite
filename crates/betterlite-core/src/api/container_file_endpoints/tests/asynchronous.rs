#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::ContainerFileReply;

use super::support::{
    HandlerForm, LIST_JSON, StubReply, TIMEOUT_SECONDS, TimeoutKind, decoding_to, file_list,
    file_object, fixture, list_call, logging, retrieve_call, timeout_failure, upload_call,
};

#[tokio::test]
async fn async_mock_matches_the_blocking_result() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = list_call();
    call.mock_response = Some(Json::Text(LIST_JSON.to_owned()));
    let blocking = endpoints
        .container_file_call("list_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .expect("the mock answers");
    let asynchronous = endpoints
        .acontainer_file_call("alist_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .await
        .expect("the mock answers");
    assert_eq!(asynchronous, blocking);
    assert_eq!(asynchronous, ContainerFileReply::List(file_list()));
    assert!(fixture.handler.seen().is_empty());
}

#[tokio::test]
async fn async_uses_the_async_handler_form() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let reply = endpoints
        .acontainer_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .await
        .expect("the handler answers");
    assert_eq!(reply, ContainerFileReply::File(file_object()));
    let recorded = fixture.handler.seen();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].form, HandlerForm::Async);
}

#[tokio::test]
async fn async_handler_matches_the_blocking_result() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let blocking = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let asynchronous = endpoints
        .acontainer_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .await
        .expect("the handler answers");
    assert_eq!(asynchronous, blocking);
    let recorded = fixture.handler.seen();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].form, HandlerForm::Blocking);
    assert_eq!(recorded[1].form, HandlerForm::Async);
}

#[tokio::test]
async fn async_prepare_error_uses_the_resolved_provider() {
    let mut fixture = fixture();
    fixture.decoder.answer = decoding_to("bogus", None);
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .acontainer_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .await
        .expect_err("the decoded provider is unknown");
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("a ValueError maps to a connection error");
    };
    assert_eq!(context.provider.as_deref(), Some("bogus"));
    assert_eq!(context.model.as_deref(), Some(""));
}

#[tokio::test]
async fn error_async_upload_timeout_maps_to_the_timeout_class() {
    let mut fixture = fixture();
    fixture
        .handler
        .fail_with(timeout_failure(TimeoutKind::Deadline).await);
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .acontainer_file_call(
            "aupload_container_file",
            &upload_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .await
        .expect_err("the provider times out");
    let ErrorClass::Timeout(details) = &failure.class else {
        panic!("a transport timeout on an async POST maps to the timeout class");
    };
    assert_eq!(details.original_status.0, 408);
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn error_async_delete_timeout_stays_a_connection_error() {
    let mut fixture = fixture();
    fixture
        .handler
        .fail_with(timeout_failure(TimeoutKind::Connect).await);
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .acontainer_file_call(
            "adelete_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .await
        .expect_err("the provider times out");
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn async_handler_error_uses_the_callers_provider() {
    let mut fixture = fixture();
    fixture.decoder.answer = decoding_to("azure", None);
    fixture.handler.reply = StubReply::Status {
        status_code: 404,
        message: "no such file".to_owned(),
    };
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .acontainer_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .await
        .expect_err("the provider fails");
    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("a 404 maps to not found");
    };
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(details.message.contains("no such file"));
    assert_eq!(failure.error.to_string(), "no such file");
}
