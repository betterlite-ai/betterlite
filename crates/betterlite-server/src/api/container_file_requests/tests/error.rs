use std::sync::Arc;

use betterlite_core::api::container_ownership::errors::ContainerOwnershipError;
use betterlite_core::api::request_form::convert::ConvertError;
use betterlite_core::api::request_form::values::FormError;

use super::super::{ContainerFileRequestError, FormReadError};
use super::support::{
    FakeAccess, FakeProcessing, caller, container_requests, file_upload_form, form, json_reply,
    request_parts, text_part, upload_part, upload_request,
};

#[tokio::test]
async fn error_form_failure_skips_the_exception_handling() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let upload = form(vec![
        upload_part("file", "data.csv"),
        upload_part("file[]", "second.csv"),
    ]);
    let payload = upload_request("aupload_container_file", upload);
    let error = requests
        .process_file_upload_request(&parts, "cntr", &caller("alice"), &payload)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ContainerFileRequestError::Form(FormReadError::Form(FormError::ScalarAppend { .. }))
    ));
    assert!(access.events().is_empty());
    assert!(processing.failures().is_empty());
}

#[tokio::test]
async fn error_convert_failure_skips_the_exception_handling() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let upload = form(vec![
        upload_part("file[]", "first.csv"),
        text_part("file[]", "not-a-file"),
    ]);
    let payload = upload_request("aupload_container_file", upload);
    let error = requests
        .process_file_upload_request(&parts, "cntr", &caller("alice"), &payload)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ContainerFileRequestError::Form(FormReadError::Convert(ConvertError::MissingRead {
            index: 1,
            ..
        }))
    ));
    assert!(access.events().is_empty());
    assert!(processing.failures().is_empty());
}

#[tokio::test]
async fn error_forwarding_failure_skips_the_exception_handling() {
    let mut fake_access = FakeAccess::granting();
    fake_access.refuse_forwarding = true;
    let access = Arc::new(fake_access);
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let payload = upload_request("aupload_container_file", file_upload_form());
    let error = requests
        .process_file_upload_request(&parts, "cntr", &caller("alice"), &payload)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ContainerFileRequestError::AccessDenied(ContainerOwnershipError::NoIdentityScope)
    ));
    assert!(processing.failures().is_empty());
}
