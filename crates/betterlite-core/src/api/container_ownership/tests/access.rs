use crate::api::response_id_codecs::container::build_container_id;

use super::super::access::assert_container_access;
use super::super::errors::ContainerOwnershipError;
use super::super::{CONTAINER_OBJECT_PURPOSE, ContainerCaller, ContainerOwnership};
use super::support::{
    Effect, ScriptedObjects, admin, advance, anonymous_admin, fixed_clock, movable_clock, principal,
};

#[tokio::test]
async fn admin_bypasses_the_owner_lookup() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let root = admin("root");
    let caller = ContainerCaller {
        principal: Some(&root),
        custom_llm_provider: "azure",
    };
    let identity = assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap();
    assert_eq!(identity.original_container_id, "container-1");
    assert_eq!(identity.custom_llm_provider, "azure");
    assert_eq!(objects.effects(), Vec::new());
}

#[tokio::test]
async fn identity_less_admin_bypasses_the_owner_lookup() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let root = anonymous_admin();
    let caller = ContainerCaller {
        principal: Some(&root),
        custom_llm_provider: "openai",
    };
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap();
    assert_eq!(objects.effects(), Vec::new());
}

#[tokio::test]
async fn owner_can_access_a_tracked_container() {
    let objects = ScriptedObjects::default();
    objects.row_for(
        "container:openai:container-1",
        CONTAINER_OBJECT_PURPOSE,
        Some("alice"),
    );
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap();
    assert_eq!(
        objects.effects(),
        vec![Effect::FindContainer(
            "container:openai:container-1".to_owned()
        )]
    );
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap();
    assert_eq!(objects.effects().len(), 1);
}

#[tokio::test]
async fn non_owner_is_forbidden() {
    let objects = ScriptedObjects::default();
    objects.row_for(
        "container:openai:container-1",
        CONTAINER_OBJECT_PURPOSE,
        Some("alice"),
    );
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let bob = principal("bob");
    let caller = ContainerCaller {
        principal: Some(&bob),
        custom_llm_provider: "openai",
    };
    let error = assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::Forbidden));
    assert_eq!(error.status_code(), 403);
    assert_eq!(error.to_string(), "Forbidden");
}

#[tokio::test]
async fn untracked_container_is_forbidden_for_a_non_admin() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let bob = principal("bob");
    let caller = ContainerCaller {
        principal: Some(&bob),
        custom_llm_provider: "openai",
    };
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap_err();
    assert_eq!(
        objects.effects(),
        vec![Effect::FindContainer(
            "container:openai:container-1".to_owned()
        )]
    );
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap_err();
    assert_eq!(objects.effects().len(), 1);
}

#[tokio::test]
async fn a_row_with_another_purpose_is_untracked() {
    let objects = ScriptedObjects::default();
    objects.row_for("container:openai:container-1", "batch", Some("alice"));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let error = assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::Forbidden));
}

#[tokio::test]
async fn access_skips_the_database_when_unavailable() {
    let objects = ScriptedObjects::unavailable();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let bob = principal("bob");
    let caller = ContainerCaller {
        principal: Some(&bob),
        custom_llm_provider: "openai",
    };
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap_err();
    assert_eq!(objects.effects(), Vec::new());
}

#[tokio::test]
async fn access_reads_the_owner_of_a_managed_id() {
    let objects = ScriptedObjects::default();
    objects.row_for(
        "container:anthropic:container-raw",
        CONTAINER_OBJECT_PURPOSE,
        Some("alice"),
    );
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let managed = build_container_id(Some("anthropic"), Some("deployment-1"), "container-raw");
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let identity = assert_container_access(&ownership, &managed, caller)
        .await
        .unwrap();
    assert_eq!(identity.original_container_id, "container-raw");
    assert_eq!(identity.custom_llm_provider, "anthropic");
    assert_eq!(
        objects.effects(),
        vec![Effect::FindContainer(
            "container:anthropic:container-raw".to_owned()
        )]
    );
}

#[tokio::test]
async fn owner_cache_expires_after_the_ttl() {
    let objects = ScriptedObjects::default();
    objects.row_for(
        "container:openai:container-1",
        CONTAINER_OBJECT_PURPOSE,
        Some("alice"),
    );
    let (clock, now) = movable_clock();
    let ownership = ContainerOwnership::new(objects.clone(), clock).unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap();
    objects.update(|state| {
        state.rows.remove("container:openai:container-1");
    });
    advance(&now, 1_060.0);
    assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap();
    assert_eq!(objects.effects().len(), 1);
    advance(&now, 1_061.0);
    let error = assert_container_access(&ownership, "container-1", caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::Forbidden));
    assert_eq!(objects.effects().len(), 2);
}
