use crate::api::response_id_codecs::container::build_container_id;

use super::super::{ContainerIdentity, decode_for_ownership, owner_scope_strings};
use super::support::{anonymous, principal};

#[test]
fn decode_keeps_a_raw_id_and_the_caller_provider() {
    let identity = decode_for_ownership("container-raw", "azure");
    assert_eq!(
        identity,
        ContainerIdentity {
            original_container_id: "container-raw".to_owned(),
            custom_llm_provider: "azure".to_owned(),
        }
    );
}

#[test]
fn decode_reads_the_provider_of_a_managed_id_for_an_openai_caller() {
    let managed = build_container_id(Some("anthropic"), Some("deployment-1"), "container-raw");
    let identity = decode_for_ownership(&managed, "openai");
    assert_eq!(identity.original_container_id, "container-raw");
    assert_eq!(identity.custom_llm_provider, "anthropic");
}

#[test]
fn decode_keeps_a_caller_provider_that_is_not_openai() {
    let managed = build_container_id(Some("anthropic"), Some("deployment-1"), "container-raw");
    let identity = decode_for_ownership(&managed, "azure");
    assert_eq!(identity.original_container_id, "container-raw");
    assert_eq!(identity.custom_llm_provider, "azure");
}

#[test]
fn decode_keeps_the_openai_caller_when_the_managed_id_has_no_provider() {
    let managed = build_container_id(None, None, "container-raw");
    let identity = decode_for_ownership(&managed, "openai");
    assert_eq!(identity.original_container_id, "container-raw");
    assert_eq!(identity.custom_llm_provider, "openai");
}

#[test]
fn owner_scopes_cover_the_user_team_org_and_keys() {
    let alice = principal("alice");
    assert_eq!(
        owner_scope_strings(Some(&alice)),
        Vec::from([
            "alice".to_owned(),
            "user:alice".to_owned(),
            "team:team-1".to_owned(),
            "org:org-1".to_owned(),
            "key:key-1".to_owned(),
            "key:token-1".to_owned(),
        ])
    );
}

#[test]
fn owner_scopes_are_empty_without_an_identity() {
    assert_eq!(owner_scope_strings(None), Vec::<String>::new());
    assert_eq!(
        owner_scope_strings(Some(&anonymous())),
        Vec::<String>::new()
    );
}
