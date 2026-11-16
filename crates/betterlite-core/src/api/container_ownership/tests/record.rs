use betterlite_models::decisions::Json;

use crate::api::response_id_codecs::container::build_container_id;
use crate::api::response_id_traversal::record::Record;

use super::super::access::assert_container_access;
use super::super::errors::ContainerOwnershipError;
use super::super::record::{record_container_owner, record_response_containers};
use super::super::response::synthetic_container;
use super::super::{CONTAINER_OBJECT_PURPOSE, ContainerCaller, ContainerOwnership};
use super::support::{
    Effect, ScriptedObjects, anonymous, anonymous_admin, capture_logs, fixed_clock, principal,
};

#[tokio::test]
async fn record_with_no_id_warns_and_skips() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let response: Record = serde_json::from_str(r#"{"object":"container"}"#).unwrap();
    let (logs, _guard) = capture_logs();
    record_container_owner(&ownership, &response, caller)
        .await
        .unwrap();
    assert_eq!(objects.effects(), Vec::new());
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("provider response has no id"));
}

#[tokio::test]
async fn record_skips_when_prisma_unavailable() {
    let objects = ScriptedObjects::unavailable();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let (logs, _guard) = capture_logs();
    record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap();
    assert_eq!(objects.effects(), Vec::new());
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("prisma_client is None"));
}

#[tokio::test]
async fn record_rejects_identity_less_caller() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let visitor = anonymous();
    let caller = ContainerCaller {
        principal: Some(&visitor),
        custom_llm_provider: "openai",
    };
    let error = record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::NoIdentityScope));
    assert_eq!(error.status_code(), 403);
    assert_eq!(
        error.to_string(),
        "Unable to record container ownership: caller has no identity scope."
    );
    assert_eq!(objects.effects(), Vec::new());
}

#[tokio::test]
async fn record_rejects_an_identity_less_admin() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let visitor = anonymous_admin();
    let caller = ContainerCaller {
        principal: Some(&visitor),
        custom_llm_provider: "openai",
    };
    let error = record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::NoIdentityScope));
    assert_eq!(error.status_code(), 403);
}

#[tokio::test]
async fn record_rejects_a_missing_principal() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let caller = ContainerCaller {
        principal: None,
        custom_llm_provider: "openai",
    };
    let error = record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::NoIdentityScope));
    assert_eq!(objects.effects(), Vec::new());
}

#[tokio::test]
async fn record_rejects_conflicting_purpose() {
    let objects = ScriptedObjects::default();
    objects.row_for("container:openai:container-1", "batch", Some("alice"));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let error = record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        ContainerOwnershipError::ConflictingPurpose
    ));
    assert_eq!(error.status_code(), 500);
    assert_eq!(error.to_string(), "Unable to track container");
    assert_eq!(
        objects.effects(),
        vec![Effect::FindUnique(
            "container:openai:container-1".to_owned()
        )]
    );
}

#[tokio::test]
async fn record_rejects_forbidden_update() {
    let objects = ScriptedObjects::default();
    objects.row_for(
        "container:openai:container-1",
        CONTAINER_OBJECT_PURPOSE,
        Some("bob"),
    );
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let error = record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::Forbidden));
    assert_eq!(error.status_code(), 403);
    assert_eq!(error.to_string(), "Forbidden");
    assert!(objects.updates().is_empty());
}

#[tokio::test]
async fn record_creates_a_row_for_a_new_container() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "azure",
    };
    record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap();
    let created = objects.created();
    assert_eq!(created.len(), 1);
    let write = &created[0];
    assert_eq!(write.unified_object_id, "container-1");
    assert_eq!(write.model_object_id, "container:azure:container-1");
    assert_eq!(write.file_purpose, CONTAINER_OBJECT_PURPOSE);
    assert_eq!(write.created_by, "alice");
    assert_eq!(write.updated_by, "alice");
    let Json::Object(fields) = &write.file_object else {
        panic!("the owner file object is a mapping");
    };
    assert_eq!(
        fields.get("custom_llm_provider"),
        Some(&Json::Text("azure".to_owned()))
    );
    assert_eq!(
        fields.get("provider_container_id"),
        Some(&Json::Text("container-1".to_owned()))
    );
}

#[tokio::test]
async fn record_warms_the_owner_cache_for_the_recorded_container() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "azure",
    };
    record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap();
    let effects = objects.effects().len();
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap();
    assert_eq!(objects.effects().len(), effects);
    let bob = principal("bob");
    let bob_caller = ContainerCaller {
        principal: Some(&bob),
        custom_llm_provider: "azure",
    };
    let error = assert_container_access(&ownership, "container-1", bob_caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::Forbidden));
    assert_eq!(objects.effects().len(), effects);
}

#[tokio::test]
async fn record_updates_a_row_owned_by_the_caller() {
    let objects = ScriptedObjects::default();
    objects.row_for(
        "container:azure:container-1",
        CONTAINER_OBJECT_PURPOSE,
        Some("alice"),
    );
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "azure",
    };
    record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap();
    assert!(objects.created().is_empty());
    let updates = objects.updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].model_object_id, "container:azure:container-1");
    assert_eq!(updates[0].unified_object_id, "container-1");
    assert_eq!(updates[0].updated_by, "alice");
}

#[tokio::test]
async fn record_keeps_a_managed_id_as_the_stored_id() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let managed = build_container_id(Some("anthropic"), Some("deployment-1"), "container-raw");
    record_container_owner(&ownership, &synthetic_container(&managed), caller)
        .await
        .unwrap();
    let created = objects.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].unified_object_id, managed);
    assert_eq!(
        created[0].model_object_id,
        "container:anthropic:container-raw"
    );
}

#[tokio::test]
async fn record_keeps_a_caller_provider_that_is_not_openai() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "azure",
    };
    let managed = build_container_id(Some("anthropic"), Some("deployment-1"), "container-raw");
    record_container_owner(&ownership, &synthetic_container(&managed), caller)
        .await
        .unwrap();
    let created = objects.created();
    assert_eq!(created[0].model_object_id, "container:azure:container-raw");
    let Json::Object(fields) = &created[0].file_object else {
        panic!("the owner file object is a mapping");
    };
    assert_eq!(
        fields.get("custom_llm_provider"),
        Some(&Json::Text("azure".to_owned()))
    );
    assert_eq!(
        fields.get("provider_container_id"),
        Some(&Json::Text("container-raw".to_owned()))
    );
}

#[tokio::test]
async fn batch_recording_survives_a_container_batch_exception() {
    let objects = ScriptedObjects::default();
    objects.row_for("container:azure:container-a", "batch", Some("alice"));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let response: Record = serde_json::from_str(
        r#"{"_hidden_params":{"custom_llm_provider":"azure"},"output":[{"type":"code_interpreter_call","container_id":"container-a"},{"type":"code_interpreter_call","container_id":"container-b"}]}"#,
    )
    .unwrap();
    let (logs, _guard) = capture_logs();
    record_response_containers(&ownership, &response, Some(&alice), None).await;
    let created = objects.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].model_object_id, "container:azure:container-b");
    let errors = logs.errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("Failed to record container ownership from responses output"));
}

#[tokio::test]
async fn batch_recording_reads_the_provider_from_the_hidden_params() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let response: Record = serde_json::from_str(
        r#"{"_hidden_params":{"custom_llm_provider":"azure"},"output":[{"container_id":"container-c"}]}"#,
    )
    .unwrap();
    record_response_containers(&ownership, &response, Some(&alice), None).await;
    let created = objects.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].model_object_id, "container:azure:container-c");
    let Json::Object(fields) = &created[0].file_object else {
        panic!("the owner file object is a mapping");
    };
    assert_eq!(
        fields.get("custom_llm_provider"),
        Some(&Json::Text("azure".to_owned()))
    );
}

#[tokio::test]
async fn batch_recording_ignores_an_empty_provider_argument() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let response: Record = serde_json::from_str(
        r#"{"_hidden_params":{"custom_llm_provider":"azure"},"output":[{"container_id":"container-c"}]}"#,
    )
    .unwrap();
    record_response_containers(&ownership, &response, Some(&alice), Some("")).await;
    let created = objects.created();
    assert_eq!(created[0].model_object_id, "container:azure:container-c");
}

#[tokio::test]
async fn batch_recording_prefers_the_caller_provider() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let response: Record = serde_json::from_str(
        r#"{"_hidden_params":{"custom_llm_provider":"azure"},"output":[{"container_id":"container-c"}]}"#,
    )
    .unwrap();
    record_response_containers(&ownership, &response, Some(&alice), Some("anthropic")).await;
    let created = objects.created();
    assert_eq!(
        created[0].model_object_id,
        "container:anthropic:container-c"
    );
}

#[tokio::test]
async fn batch_recording_skips_a_response_without_containers() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let response: Record = serde_json::from_str(r#"{"id":"resp_1","output":[]}"#).unwrap();
    let (logs, _guard) = capture_logs();
    record_response_containers(&ownership, &response, Some(&alice), None).await;
    assert_eq!(objects.effects(), Vec::new());
    assert!(logs.messages().is_empty());
}
