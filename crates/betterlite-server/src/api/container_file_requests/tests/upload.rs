use std::sync::Arc;

use betterlite_core::api::container_ownership::ContainerForwardingParams;
use betterlite_core::api::request_form::convert::FileTuple;
use betterlite_core::api::request_form::values::FormValue;

use super::super::{
    ContainerFileRequestError, ContainerFormValue, ContainerRequestData, RequestFailure,
};
use super::support::{
    FakeAccess, FakeProcessing, caller, container_requests, file_upload_form, form, json_reply,
    proxy_failure, request_parts, text_part, upload_part,
};

#[tokio::test]
async fn upload_missing_file_fails_before_access() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let upload = form(vec![text_part("purpose", "assistants")]);
    let error = requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &upload,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ContainerFileRequestError::MissingFileField));
    assert!(access.events().is_empty());
    assert!(processing.processed().is_empty());
    assert!(processing.failures().is_empty());
}

#[tokio::test]
async fn upload_single_file_becomes_the_tuple() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &file_upload_form(),
        )
        .await
        .unwrap();
    let calls = processing.processed();
    let ContainerRequestData::Form(data) = &calls[0].data else {
        panic!("the upload data is a form");
    };
    let file = FileTuple {
        filename: "data.csv".to_owned(),
        content: b"data.csv".to_vec(),
        content_type: Some("text/csv".to_owned()),
    };
    assert_eq!(data.get("file"), Some(&ContainerFormValue::Single(file)));
    assert_eq!(
        data.get("purpose"),
        Some(&ContainerFormValue::Field(FormValue::Text(
            "assistants".to_owned()
        )))
    );
}

#[tokio::test]
async fn upload_file_list_keeps_only_the_first_tuple() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let upload = form(vec![
        text_part("purpose", "assistants"),
        upload_part("file[]", "first.csv"),
        upload_part("file[]", "second.csv"),
    ]);
    requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &upload,
        )
        .await
        .unwrap();
    let calls = processing.processed();
    let ContainerRequestData::Form(data) = &calls[0].data else {
        panic!("the upload data is a form");
    };
    let file = FileTuple {
        filename: "first.csv".to_owned(),
        content: b"first.csv".to_vec(),
        content_type: Some("text/csv".to_owned()),
    };
    assert_eq!(data.get("file"), Some(&ContainerFormValue::Single(file)));
}

#[tokio::test]
async fn upload_text_file_field_stays_text() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let upload = form(vec![text_part("file", "just-text")]);
    requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &upload,
        )
        .await
        .unwrap();
    let calls = processing.processed();
    let ContainerRequestData::Form(data) = &calls[0].data else {
        panic!("the upload data is a form");
    };
    assert_eq!(
        data.get("file"),
        Some(&ContainerFormValue::Field(FormValue::Text(
            "just-text".to_owned()
        )))
    );
}

#[tokio::test]
async fn upload_forwarding_overwrites_form_fields() {
    let mut fake_access = FakeAccess::granting();
    fake_access.forwarding = ContainerForwardingParams {
        container_id: "cntr-upstream".to_owned(),
        custom_llm_provider: "bedrock".to_owned(),
        model_id: Some("deployment-7".to_owned()),
    };
    let access = Arc::new(fake_access);
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let upload = form(vec![
        text_part("container_id", "form-container"),
        text_part("custom_llm_provider", "form-provider"),
        text_part("model_id", "form-model"),
        text_part("purpose", "assistants"),
        upload_part("file", "data.csv"),
    ]);
    requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &upload,
        )
        .await
        .unwrap();
    let calls = processing.processed();
    let ContainerRequestData::Form(data) = &calls[0].data else {
        panic!("the upload data is a form");
    };
    let keys: Vec<&str> = data.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "container_id",
            "custom_llm_provider",
            "model_id",
            "purpose",
            "file"
        ]
    );
    assert_eq!(
        data.get("container_id"),
        Some(&ContainerFormValue::Field(FormValue::Text(
            "cntr-upstream".to_owned()
        )))
    );
    assert_eq!(
        data.get("custom_llm_provider"),
        Some(&ContainerFormValue::Field(FormValue::Text(
            "bedrock".to_owned()
        )))
    );
    assert_eq!(
        data.get("model_id"),
        Some(&ContainerFormValue::Field(FormValue::Text(
            "deployment-7".to_owned()
        )))
    );
    assert_eq!(
        data.get("purpose"),
        Some(&ContainerFormValue::Field(FormValue::Text(
            "assistants".to_owned()
        )))
    );
}

#[tokio::test]
async fn upload_keeps_the_form_model_id_when_forwarding_has_none() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let upload = form(vec![
        text_part("model_id", "form-model"),
        upload_part("file", "data.csv"),
    ]);
    requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &upload,
        )
        .await
        .unwrap();
    let calls = processing.processed();
    let ContainerRequestData::Form(data) = &calls[0].data else {
        panic!("the upload data is a form");
    };
    assert_eq!(
        data.get("model_id"),
        Some(&ContainerFormValue::Field(FormValue::Text(
            "form-model".to_owned()
        )))
    );
}

#[tokio::test]
async fn upload_returns_the_processing_reply_unchanged() {
    let access = Arc::new(FakeAccess::granting());
    let reply = json_reply();
    let processing = Arc::new(FakeProcessing::replying(reply.clone()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let answer = requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &file_upload_form(),
        )
        .await
        .unwrap();
    assert_eq!(answer, reply);
    let calls = processing.processed();
    assert_eq!(calls[0].route_type, "aupload_container_file");
    assert_eq!(calls[0].model, None);
}

#[tokio::test]
async fn upload_process_failure_fails_through_the_exception_handling() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::failing(proxy_failure()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let error = requests
        .process_file_upload_request(
            &parts,
            "cntr",
            "aupload_container_file",
            &caller("alice"),
            &file_upload_form(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ContainerFileRequestError::Processing(_)));
    assert_eq!(
        processing.failures(),
        [RequestFailure::Process(proxy_failure())]
    );
}
