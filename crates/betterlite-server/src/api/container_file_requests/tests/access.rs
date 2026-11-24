use std::sync::Arc;

use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_core::api::container_ownership::ContainerOwnership;
use betterlite_core::api::container_ownership::errors::ContainerOwnershipError;
use betterlite_core::api::container_ownership::repository::ContainerOwnerRow;
use betterlite_core::api::container_ownership::{ContainerForwardingParams, ContainerIdentity};
use betterlite_prompts::arguments::json::Json;

use super::super::ports::{ContainerAccess as _, OwnershipAccess};
use super::super::{ContainerFileRequestError, ContainerRequestData};
use super::support::{
    FakeAccess, FakeObjects, FakeProcessing, bytes_reply, caller, container_requests,
    file_upload_form, json_reply, request_parts, upload_request,
};

fn native_adapter() -> OwnershipAccess<FakeObjects> {
    let objects = FakeObjects::default();
    objects.rows.lock().unwrap().insert(
        "container:openai:cntr_native".to_owned(),
        ContainerOwnerRow {
            unified_object_id: None,
            model_object_id: "container:openai:cntr_native".to_owned(),
            file_purpose: "container".to_owned(),
            created_by: Some("alice".to_owned()),
        },
    );
    let clock: Clock = Arc::new(|| 1_000.0_f64);
    OwnershipAccess {
        ownership: ContainerOwnership::new(objects, clock).unwrap(),
    }
}

#[tokio::test]
async fn access_check_runs_before_forwarding_with_the_path_id() {
    let access = Arc::new(FakeAccess::granting());
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png")));
    let requests = container_requests(Arc::clone(&access), processing);
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", Some("azure"));
    requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("alice"))
        .await
        .unwrap();
    assert_eq!(
        access.events(),
        [
            "check:cntr:alice:azure".to_owned(),
            "forward:cntr:cntr-upstream:openai".to_owned()
        ]
    );
}

#[tokio::test]
async fn access_resolved_identity_replaces_the_path_values() {
    let mut fake_access = FakeAccess::granting();
    fake_access.identity = ContainerIdentity {
        original_container_id: "cntr-decoded".to_owned(),
        custom_llm_provider: "bedrock".to_owned(),
    };
    fake_access.forwarding = ContainerForwardingParams {
        container_id: "cntr-decoded".to_owned(),
        custom_llm_provider: "bedrock".to_owned(),
        model_id: Some("deployment-7".to_owned()),
    };
    let access = Arc::new(fake_access);
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png")));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr-managed/files/file-1/content", None);
    requests
        .process_file_content_request(&parts, "cntr-managed", "file-1", &caller("alice"))
        .await
        .unwrap();
    let calls = processing.processed();
    assert_eq!(calls.len(), 1);
    let ContainerRequestData::Json(object) = &calls[0].data else {
        panic!("the content data is a JSON object");
    };
    assert_eq!(
        object.get("container_id"),
        Some(&Json::Text("cntr-decoded".to_owned()))
    );
    assert_eq!(
        object.get("custom_llm_provider"),
        Some(&Json::Text("bedrock".to_owned()))
    );
}

#[tokio::test]
async fn access_denial_returns_before_processing() {
    let mut fake_access = FakeAccess::granting();
    fake_access.deny = true;
    let access = Arc::new(fake_access);
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png")));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", None);
    let error = requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("bob"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ContainerFileRequestError::AccessDenied(ContainerOwnershipError::Forbidden)
    ));
    assert!(processing.processed().is_empty());
    assert!(processing.failures().is_empty());
    assert_eq!(access.events(), ["check:cntr:bob:openai".to_owned()]);
}

#[tokio::test]
async fn access_upload_denial_skips_the_processing() {
    let mut fake_access = FakeAccess::granting();
    fake_access.deny = true;
    let access = Arc::new(fake_access);
    let processing = Arc::new(FakeProcessing::replying(json_reply()));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files", None);
    let payload = upload_request("aupload_container_file", file_upload_form());
    let error = requests
        .process_file_upload_request(&parts, "cntr", &caller("bob"), &payload)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ContainerFileRequestError::AccessDenied(ContainerOwnershipError::Forbidden)
    ));
    assert!(processing.processed().is_empty());
}

#[tokio::test]
async fn access_forwarding_failure_returns_as_is() {
    let mut fake_access = FakeAccess::granting();
    fake_access.refuse_forwarding = true;
    let access = Arc::new(fake_access);
    let processing = Arc::new(FakeProcessing::replying(bytes_reply(b"png")));
    let requests = container_requests(Arc::clone(&access), Arc::clone(&processing));
    let parts = request_parts("/v1/containers/cntr/files/file-1/content", None);
    let error = requests
        .process_file_content_request(&parts, "cntr", "file-1", &caller("alice"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ContainerFileRequestError::AccessDenied(ContainerOwnershipError::NoIdentityScope)
    ));
    assert!(processing.failures().is_empty());
}

#[tokio::test]
async fn access_adapter_checks_b3362_ownership() {
    let adapter = native_adapter();
    let identity = adapter
        .check_access("cntr_native", &caller("alice"), "openai")
        .await
        .unwrap();
    assert_eq!(
        identity,
        ContainerIdentity {
            original_container_id: "cntr_native".to_owned(),
            custom_llm_provider: "openai".to_owned(),
        }
    );
    let forwarding = adapter
        .forwarding_params(
            "cntr_native",
            &identity.original_container_id,
            &identity.custom_llm_provider,
        )
        .await
        .unwrap();
    assert_eq!(
        forwarding,
        ContainerForwardingParams {
            container_id: "cntr_native".to_owned(),
            custom_llm_provider: "openai".to_owned(),
            model_id: None,
        }
    );
    let denied = adapter
        .check_access("cntr_native", &caller("bob"), "openai")
        .await;
    assert!(matches!(denied, Err(ContainerOwnershipError::Forbidden)));
}
