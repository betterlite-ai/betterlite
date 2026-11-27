use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;

use crate::api::vector_file_resolution::error::VectorFileResolutionError;
use crate::api::vector_file_resolution::store_overlay::unified_store_and_registry_overlay;

use super::support::{encoded, object, store, store_deps};

const MANAGED_STORE: &str =
    r#"{"vector_store_id":"vs_1","custom_llm_provider":"azure","litellm_credential_name":"cred"}"#;

#[test]
fn overlay_unified_model_id_before_targets() {
    let deps = store_deps(None, None, true);
    let mut data = object(r#"{"vector_store_id":"unified"}"#);
    let id = encoded(
        "litellm_proxy:vector_store;model_id,m1;target_model_names,m2,m3;resource_id,vs_provider",
    );
    unified_store_and_registry_overlay(&mut data, &id, &deps).unwrap();
    assert_eq!(
        data,
        object(r#"{"model":"m1","vector_store_id":"vs_provider"}"#)
    );
}

#[test]
fn overlay_unified_targets_when_model_id_blank() {
    let deps = store_deps(None, None, true);
    let mut data = object(r#"{"vector_store_id":"unified"}"#);
    let id = encoded("litellm_proxy:vector_store;model_id,  ;target_model_names,m2,m3");
    unified_store_and_registry_overlay(&mut data, &id, &deps).unwrap();
    assert_eq!(
        data,
        object(r#"{"model":"m2","vector_store_id":"unified"}"#)
    );
}

#[test]
fn overlay_unified_blank_parts_leave_data() {
    let deps = store_deps(None, None, true);
    let mut data = object(r#"{"vector_store_id":"unified"}"#);
    let id = encoded("litellm_proxy:vector_store;model_id,;target_model_names,,m2;resource_id,   ");
    unified_store_and_registry_overlay(&mut data, &id, &deps).unwrap();
    assert_eq!(data, object(r#"{"vector_store_id":"unified"}"#));
}

#[test]
fn overlay_unified_skips_registry() {
    let mut registry = VectorStoreRegistry::default();
    registry.stores.push(store(
        r#"{"vector_store_id":"unified_id","custom_llm_provider":"openai"}"#,
    ));
    let deps = store_deps(None, Some(&registry), true);
    let mut first_data = object(r#"{"vector_store_id":"unified"}"#);
    let first_id = encoded("litellm_proxy:vector_store;model_id,m1");
    unified_store_and_registry_overlay(&mut first_data, &first_id, &deps).unwrap();
    assert_eq!(
        first_data,
        object(r#"{"model":"m1","vector_store_id":"unified"}"#)
    );
    let mut second_data = object(r#"{"vector_store_id":"unified"}"#);
    let second_id = encoded("litellm_proxy:vector_store");
    unified_store_and_registry_overlay(&mut second_data, &second_id, &deps).unwrap();
    assert_eq!(second_data, object(r#"{"vector_store_id":"unified"}"#));
}

#[test]
fn overlay_managed_store_wins() {
    let managed = store(MANAGED_STORE);
    let mut registry = VectorStoreRegistry::default();
    registry.stores.push(store(
        r#"{"vector_store_id":"vs_1","custom_llm_provider":"openai","litellm_credential_name":"other"}"#,
    ));
    let deps = store_deps(Some(&managed), Some(&registry), true);
    let mut data = object(r#"{"vector_store_id":"vs_1"}"#);
    unified_store_and_registry_overlay(&mut data, "vs_1", &deps).unwrap();
    assert_eq!(
        data,
        object(
            r#"{"vector_store_id":"vs_1","custom_llm_provider":"azure","litellm_credential_name":"cred"}"#
        )
    );
}

#[test]
fn overlay_registry_lookup_when_allowed() {
    let mut registry = VectorStoreRegistry::default();
    registry.stores.push(store(MANAGED_STORE));
    let deps = store_deps(None, Some(&registry), true);
    let mut data = object("{}");
    unified_store_and_registry_overlay(&mut data, "vs_1", &deps).unwrap();
    assert_eq!(
        data,
        object(r#"{"custom_llm_provider":"azure","litellm_credential_name":"cred"}"#)
    );
}

#[test]
fn overlay_registry_skipped_when_lookup_disabled() {
    let mut registry = VectorStoreRegistry::default();
    registry.stores.push(store(MANAGED_STORE));
    let deps = store_deps(None, Some(&registry), false);
    let mut data = object("{}");
    unified_store_and_registry_overlay(&mut data, "vs_1", &deps).unwrap();
    assert_eq!(data, object("{}"));
}

#[test]
fn overlay_nulls_copied_and_params_override() {
    let managed = store(
        r#"{"vector_store_id":"vs_1","custom_llm_provider":null,"litellm_credential_name":null,"litellm_params":{"custom_llm_provider":"openai","api_key":"sk-params","model":"m_params"}}"#,
    );
    let deps = store_deps(Some(&managed), None, true);
    let mut data = object(r#"{"model":"request","other":"x"}"#);
    unified_store_and_registry_overlay(&mut data, "vs_1", &deps).unwrap();
    assert_eq!(
        data,
        object(
            r#"{"custom_llm_provider":"openai","litellm_credential_name":null,"api_key":"sk-params","model":"m_params","other":"x"}"#
        )
    );
}

#[test]
fn overlay_absent_params_noop() {
    let managed = store(r#"{"vector_store_id":"vs_1","litellm_params":null}"#);
    let deps = store_deps(Some(&managed), None, true);
    let mut data = object(r#"{"other":"x"}"#);
    unified_store_and_registry_overlay(&mut data, "vs_1", &deps).unwrap();
    assert_eq!(data, object(r#"{"other":"x"}"#));
}

#[test]
fn overlay_non_object_params_error() {
    let managed = store(r#"{"vector_store_id":"vs_1","litellm_params":"text"}"#);
    let deps = store_deps(Some(&managed), None, true);
    let mut data = object(r#"{"other":"x"}"#);
    let outcome = unified_store_and_registry_overlay(&mut data, "vs_1", &deps);
    assert!(matches!(
        outcome,
        Err(VectorFileResolutionError::StoreParams)
    ));
    assert_eq!(data, object(r#"{"other":"x"}"#));
}
