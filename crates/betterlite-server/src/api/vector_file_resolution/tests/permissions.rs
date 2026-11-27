use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_body::JsonObject as RequestBody;
use betterlite_core::api::vector_endpoint_access::error::VectorEndpointAccessError;
use betterlite_models::identity::provider_id::ProviderId;
use http::HeaderMap;
use rstest::rstest;

use crate::api::vector_file_resolution::Caller;
use crate::api::vector_file_resolution::provider_permissions::provider_and_index_permissions;

use super::support::{FILE_PATH, Probe, TABLES, gated_auth, header_map, object, principal};

#[test]
fn permissions_provider_sources() {
    let auth = gated_auth(None, None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let data_provider = object(r#"{"custom_llm_provider":"azure"}"#);
    assert_eq!(
        provider_and_index_permissions(&data_provider, "vs_1", &facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Azure)
    );
    let fallback_data = object(r#"{"custom_llm_provider":null}"#);
    assert_eq!(
        provider_and_index_permissions(&fallback_data, "vs_1", &facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Openai)
    );
    let mut query_probe = Probe::get();
    query_probe.query = Some("custom_llm_provider=bedrock");
    let query_facts = query_probe.facts(FILE_PATH);
    let query_data = object("{}");
    assert_eq!(
        provider_and_index_permissions(&query_data, "vs_1", &query_facts, &caller, &TABLES)
            .unwrap(),
        Some(ProviderId::Bedrock)
    );
}

#[rstest]
#[case::empty_text(r#"{"custom_llm_provider":""}"#)]
#[case::zero(r#"{"custom_llm_provider":0}"#)]
#[case::false_value(r#"{"custom_llm_provider":false}"#)]
#[case::empty_list(r#"{"custom_llm_provider":[]}"#)]
#[case::empty_object(r#"{"custom_llm_provider":{}}"#)]
#[test]
fn permissions_falsy_data_falls_through(#[case] text: &'static str) {
    let auth = gated_auth(None, None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let mut probe = Probe::get();
    probe.headers = header_map(&[("custom-llm-provider", "azure")]);
    probe.query = Some("custom_llm_provider=bedrock");
    let facts = probe.facts(FILE_PATH);
    let candidate = object(text);
    assert_eq!(
        provider_and_index_permissions(&candidate, "vs_1", &facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Azure),
        "{text}"
    );
}

#[test]
fn permissions_empty_header_falls_to_query() {
    let auth = gated_auth(None, None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let mut probe = Probe::get();
    probe.headers = header_map(&[("custom-llm-provider", "")]);
    probe.query = Some("custom_llm_provider=bedrock&custom_llm_provider=azure");
    let facts = probe.facts(FILE_PATH);
    let data = object("{}");
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Azure)
    );
}

#[test]
fn permissions_body_only_for_writes() {
    let auth = gated_auth(None, None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let body = RequestBody::from([(
        String::from("custom_llm_provider"),
        JsonValue::String(String::from("azure")),
    )]);
    let mut probe = Probe {
        method: "POST",
        query: None,
        headers: HeaderMap::new(),
        body,
    };
    let data = object("{}");
    let post_facts = probe.facts(FILE_PATH);
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &post_facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Azure)
    );
    probe.method = "GET";
    let get_facts = probe.facts(FILE_PATH);
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &get_facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Openai)
    );
    probe.method = "DELETE";
    let delete_facts = probe.facts(FILE_PATH);
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &delete_facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Openai)
    );
}

#[test]
fn permissions_body_null_defaults_openai() {
    let auth = gated_auth(None, None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let body = RequestBody::from([(String::from("custom_llm_provider"), JsonValue::Null)]);
    let probe = Probe {
        method: "PATCH",
        query: None,
        headers: HeaderMap::new(),
        body,
    };
    let facts = probe.facts(FILE_PATH);
    let data = object("{}");
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Openai)
    );
}

#[test]
fn permissions_empty_or_unknown_is_none() {
    let auth = gated_auth(None, None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let mut probe = Probe::get();
    probe.query = Some("custom_llm_provider=");
    let facts = probe.facts(FILE_PATH);
    let empty_query_data = object("{}");
    assert_eq!(
        provider_and_index_permissions(&empty_query_data, "vs_1", &facts, &caller, &TABLES)
            .unwrap(),
        None
    );
    let unknown_data = object(r#"{"custom_llm_provider":"bogus"}"#);
    assert_eq!(
        provider_and_index_permissions(&unknown_data, "vs_1", &facts, &caller, &TABLES).unwrap(),
        None
    );
    let number_data = object(r#"{"custom_llm_provider":5}"#);
    assert_eq!(
        provider_and_index_permissions(&number_data, "vs_1", &facts, &caller, &TABLES).unwrap(),
        None
    );
}

#[test]
fn permissions_metadata_gate_allows_read_denies_write() {
    let auth = gated_auth(
        Some(
            r#"{"allowed_vector_store_indexes":[{"index_name":"vs_1","index_permissions":["read"]}]}"#,
        ),
        None,
    );
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let mut probe = Probe::get();
    let data = object(r#"{"custom_llm_provider":"openai"}"#);
    let read_facts = probe.facts("/v1/vector_stores/vs_1/files/f1");
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &read_facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Openai)
    );
    probe.method = "POST";
    let post_facts = probe.facts(FILE_PATH);
    let outcome = provider_and_index_permissions(&data, "vs_1", &post_facts, &caller, &TABLES);
    let Err(denied) = outcome else {
        panic!("a read-only grant must deny the write")
    };
    assert!(matches!(
        denied,
        VectorEndpointAccessError::FileDenied { .. }
    ));
    assert_eq!(denied.status(), 403);
}

#[test]
fn permissions_empty_allowlist_does_not_gate() {
    let auth = gated_auth(Some(r#"{"allowed_vector_store_indexes":[]}"#), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let mut probe = Probe::get();
    probe.method = "POST";
    let facts = probe.facts(FILE_PATH);
    let data = object(r#"{"custom_llm_provider":"openai"}"#);
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Openai)
    );
}

#[test]
fn permissions_team_metadata_write_allowed() {
    let team = gated_auth(
        None,
        Some(
            r#"{"allowed_vector_store_indexes":[{"index_name":"vs_1","index_permissions":["write"]}]}"#,
        ),
    );
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &team,
    };
    let mut probe = Probe::get();
    probe.method = "POST";
    let facts = probe.facts(FILE_PATH);
    let data = object(r#"{"custom_llm_provider":"openai"}"#);
    assert_eq!(
        provider_and_index_permissions(&data, "vs_1", &facts, &caller, &TABLES).unwrap(),
        Some(ProviderId::Openai)
    );
}

#[test]
fn permissions_team_metadata_read_denied() {
    let team = gated_auth(
        None,
        Some(
            r#"{"allowed_vector_store_indexes":[{"index_name":"vs_1","index_permissions":["read"]}]}"#,
        ),
    );
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &team,
    };
    let mut probe = Probe::get();
    probe.method = "POST";
    let facts = probe.facts(FILE_PATH);
    let data = object(r#"{"custom_llm_provider":"openai"}"#);
    let outcome = provider_and_index_permissions(&data, "vs_1", &facts, &caller, &TABLES);
    let Err(denied) = outcome else {
        panic!("a read-only grant must deny the write")
    };
    assert!(matches!(
        denied,
        VectorEndpointAccessError::FileDenied { vector_store_id } if vector_store_id == "vs_1"
    ));
    assert_eq!(denied.status(), 403);
}
