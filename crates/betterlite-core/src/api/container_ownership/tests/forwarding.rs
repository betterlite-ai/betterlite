use crate::api::response_id_codecs::container::build_container_id;

use super::super::access::container_owner;
use super::super::forwarding::{forwarding_params, stored_container_id};
use super::super::{CONTAINER_OBJECT_PURPOSE, ContainerOwnership, decode_for_ownership};
use super::support::{Effect, ScriptedObjects, fixed_clock};

#[tokio::test]
async fn forwarding_uses_the_decoded_model_id() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let managed = build_container_id(Some("azure"), Some("deployment-1"), "container-raw");
    let identity = decode_for_ownership(&managed, "openai");
    let params = forwarding_params(&ownership, &managed, &identity)
        .await
        .unwrap();
    assert_eq!(params.container_id, "container-raw");
    assert_eq!(params.custom_llm_provider, "azure");
    assert_eq!(params.model_id, Some("deployment-1".to_owned()));
    assert_eq!(objects.effects(), Vec::new());
}

#[tokio::test]
async fn forwarding_recovers_the_model_from_the_stored_id() {
    let objects = ScriptedObjects::default();
    let native = "container-native";
    let key = format!("{CONTAINER_OBJECT_PURPOSE}:azure:{native}");
    let stored = build_container_id(Some("azure"), Some("deployment-2"), native);
    objects.row_for(&key, CONTAINER_OBJECT_PURPOSE, Some("alice"));
    objects.set_stored_id(&key, Some(&stored));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let identity = decode_for_ownership(native, "azure");
    let params = forwarding_params(&ownership, native, &identity)
        .await
        .unwrap();
    assert_eq!(params.model_id, Some("deployment-2".to_owned()));
    assert_eq!(objects.effects(), vec![Effect::FindContainer(key)]);
}

#[tokio::test]
async fn forwarding_ignores_a_stored_id_equal_to_the_supplied_id() {
    let objects = ScriptedObjects::default();
    let native = "container-native";
    let key = format!("{CONTAINER_OBJECT_PURPOSE}:azure:{native}");
    objects.row_for(&key, CONTAINER_OBJECT_PURPOSE, Some("alice"));
    objects.set_stored_id(&key, Some(native));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let identity = decode_for_ownership(native, "azure");
    let params = forwarding_params(&ownership, native, &identity)
        .await
        .unwrap();
    assert_eq!(params.model_id, None);
}

#[tokio::test]
async fn forwarding_caches_an_absent_stored_id() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let identifier = decode_for_ownership("container-native", "azure");
    assert_eq!(
        stored_container_id(&ownership, &identifier).await.unwrap(),
        None
    );
    assert_eq!(
        objects.effects(),
        vec![Effect::FindContainer(
            "container:azure:container-native".to_owned()
        )]
    );
    assert_eq!(
        stored_container_id(&ownership, &identifier).await.unwrap(),
        None
    );
    assert_eq!(objects.effects().len(), 1);
}

#[tokio::test]
async fn forwarding_returns_no_model_when_the_stored_id_has_none() {
    let objects = ScriptedObjects::default();
    let native = "container-native";
    let key = format!("{CONTAINER_OBJECT_PURPOSE}:azure:{native}");
    let stored = build_container_id(Some("azure"), None, native);
    objects.row_for(&key, CONTAINER_OBJECT_PURPOSE, Some("alice"));
    objects.set_stored_id(&key, Some(&stored));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let identity = decode_for_ownership(native, "azure");
    let params = forwarding_params(&ownership, native, &identity)
        .await
        .unwrap();
    assert_eq!(params.model_id, None);
}

#[tokio::test]
async fn forwarding_treats_an_empty_row_value_as_absent() {
    let objects = ScriptedObjects::default();
    let key = "container:azure:container-native";
    objects.row_for(key, CONTAINER_OBJECT_PURPOSE, Some("alice"));
    objects.set_stored_id(key, Some(""));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let identity = decode_for_ownership("container-native", "azure");
    assert_eq!(
        stored_container_id(&ownership, &identity).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn forwarding_skips_the_stored_lookup_when_unavailable() {
    let objects = ScriptedObjects::unavailable();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let identity = decode_for_ownership("container-native", "azure");
    let params = forwarding_params(&ownership, "container-native", &identity)
        .await
        .unwrap();
    assert_eq!(params.model_id, None);
    assert_eq!(objects.effects(), Vec::new());
}

#[tokio::test]
async fn forwarding_reuses_the_stored_id_from_an_owner_lookup() {
    let objects = ScriptedObjects::default();
    let native = "container-native";
    let key = format!("{CONTAINER_OBJECT_PURPOSE}:azure:{native}");
    let stored = build_container_id(Some("azure"), Some("deployment-3"), native);
    objects.row_for(&key, CONTAINER_OBJECT_PURPOSE, Some("alice"));
    objects.set_stored_id(&key, Some(&stored));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let identity = decode_for_ownership(native, "azure");
    assert_eq!(
        container_owner(&ownership, &identity).await.unwrap(),
        Some("alice".to_owned())
    );
    let params = forwarding_params(&ownership, native, &identity)
        .await
        .unwrap();
    assert_eq!(params.model_id, Some("deployment-3".to_owned()));
    assert_eq!(objects.effects().len(), 1);
}
