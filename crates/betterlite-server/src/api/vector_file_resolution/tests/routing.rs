use betterlite_core::api::managed_resource_ids::decode_unified_id;

use crate::api::vector_file_resolution::Caller;
use crate::api::vector_file_resolution::file_routing::unified_file_routing;

use super::support::{
    Catalog, Checker, FILE_PATH, Grants, LookupCall, Probe, auth, authorization_failure, encoded,
    header_map, lookup_failure, managed_failure, object, principal, routing_deps,
};

const UNIFIED_FILE: &str = "litellm_proxy:file;unified_id,u1;target_model_names,m1,m2;llm_output_file_id, file_a ;llm_output_file_model_id,openai/gpt-4o";
const UNIFIED_TARGETS_ONLY: &str = "litellm_proxy:file;unified_id,u2;target_model_names,m1,m2;llm_output_file_model_id,openai/gpt-4o";
const UNIFIED_EMPTY: &str = "litellm_proxy:file";
const LEGACY_PAYLOAD: &str = "litellm:file_raw;model,m_embedded";

fn legacy_id() -> String {
    format!("file-{}", encoded(LEGACY_PAYLOAD))
}

#[tokio::test]
async fn unified_file_routing_gate_raw_id_400() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(None, &grants, None, true);
    let mut data = object(r#"{"file_id":"raw_a"}"#);
    let outcome = unified_file_routing(&mut data, "raw_a", &facts, &caller, &deps).await;
    let (status, detail) = managed_failure(outcome);
    assert_eq!(status, 400);
    assert_eq!(
        detail,
        "Raw provider file ids cannot be used when require_managed_files is enabled in litellm_settings. Use the LiteLLM managed file id returned when the file was created."
    );
    assert_eq!(data, object(r#"{"file_id":"raw_a"}"#));
}

#[tokio::test]
async fn unified_file_routing_gate_managed_id_without_checker_500() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(None, &grants, None, true);
    let file_id = encoded(UNIFIED_FILE);
    let mut data = object("{}");
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    let (status, detail) = managed_failure(outcome);
    assert_eq!(status, 500);
    assert_eq!(
        detail,
        "Managed resource ownership validation is unavailable."
    );
}

#[tokio::test]
async fn unified_file_routing_gate_denied_403() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let checker = Checker::default();
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(None, &grants, Some(&checker), true);
    let file_id = encoded(UNIFIED_FILE);
    let mut data = object("{}");
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    let (status, detail) = managed_failure(outcome);
    assert_eq!(status, 403);
    assert_eq!(
        detail,
        "The caller does not have access to this managed file id."
    );
    assert_eq!(checker.calls(), vec![file_id]);
}

#[tokio::test]
async fn unified_file_routing_gate_allowed_routes() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let checker = Checker::allowing();
    let catalog = Catalog::with(&[("m1", r#"{"api_key":"sk-m1"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, Some(&checker), true);
    let file_id = encoded(UNIFIED_FILE);
    let mut data = object(r#"{"file_id":"<id>"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id.clone()));
    assert_eq!(
        data,
        object(r#"{"model":"m1","api_key":"sk-m1","file_id":"file_a"}"#)
    );
    assert_eq!(checker.calls(), vec![file_id]);
}

#[tokio::test]
async fn unified_file_routing_unified_id_routes_credentials_and_restores() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[(
        "m1",
        r#"{"api_key":"sk-m1","custom_llm_provider":"openai"}"#,
    )]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let file_id = encoded(UNIFIED_FILE);
    assert_eq!(decode_unified_id(&file_id).as_deref(), Some(UNIFIED_FILE));
    let mut data = object(r#"{"file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(
        data,
        object(r#"{"model":"m1","api_key":"sk-m1","file_id":"file_a"}"#)
    );
    assert_eq!(
        catalog.lookups(),
        vec![LookupCall {
            model_id: String::from("m1"),
            team_id: None,
        }]
    );
}

#[tokio::test]
async fn unified_file_routing_unified_id_without_credentials_replaces_file_id() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(None, &grants, None, false);
    let file_id = encoded(UNIFIED_FILE);
    let mut data = object(r#"{"file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(data, object(r#"{"model":"m1","file_id":"file_a"}"#));
}

#[tokio::test]
async fn unified_file_routing_unified_id_without_provider_id_keeps_file_id() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m1", r#"{"api_key":"sk-m1"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let file_id = encoded(UNIFIED_TARGETS_ONLY);
    let mut data = object(r#"{"file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(
        data,
        object(r#"{"model":"m1","api_key":"sk-m1","file_id":"request"}"#)
    );
}

#[tokio::test]
async fn unified_file_routing_unified_id_without_targets_leaves_data() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m1", r#"{"api_key":"sk-m1"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let file_id = encoded(UNIFIED_EMPTY);
    let mut data = object(r#"{"file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(data, object(r#"{"file_id":"request"}"#));
    assert!(catalog.lookups().is_empty());
}

#[tokio::test]
async fn unified_file_routing_empty_credentials_skip_prepare() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m1", "{}")]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let file_id = encoded(UNIFIED_FILE);
    let mut data = object(r#"{"custom_llm_provider":"azure","file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(
        data,
        object(r#"{"model":"m1","custom_llm_provider":"azure","file_id":"file_a"}"#)
    );
}

#[tokio::test]
async fn unified_file_routing_blank_provider_id_keeps_file_id() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(None, &grants, None, false);
    let file_id = encoded("litellm_proxy:file;target_model_names,m1;llm_output_file_id,   ;");
    let mut data = object(r#"{"file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(data, object(r#"{"model":"m1","file_id":"request"}"#));
}

#[tokio::test]
async fn unified_file_routing_blank_provider_id_still_applies_credentials() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m1", r#"{"api_key":"sk-m1"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let file_id = encoded("litellm_proxy:file;target_model_names,m1;llm_output_file_id,   ;");
    let mut data = object(r#"{"file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(
        data,
        object(r#"{"model":"m1","api_key":"sk-m1","file_id":""}"#)
    );
}

#[tokio::test]
async fn unified_file_routing_skips_empty_provider_id_field() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(None, &grants, None, false);
    let file_id = encoded("litellm_proxy:file;llm_output_file_id,;llm_output_file_id,file_b");
    let mut data = object(r#"{"file_id":"request"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(data, object(r#"{"file_id":"file_b"}"#));
}

#[tokio::test]
async fn unified_file_routing_legacy_embedded_model_routes() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_embedded", r#"{"api_key":"sk-e"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let file_id = legacy_id();
    let mut data = object(r#"{"model":"m_body"}"#);
    let outcome = unified_file_routing(&mut data, &file_id, &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(file_id));
    assert_eq!(
        data,
        object(r#"{"model":"m_body","api_key":"sk-e","file_id":"file_raw"}"#)
    );
    assert_eq!(grants.asked(), vec!["m_embedded"]);
    assert_eq!(
        catalog.lookups(),
        vec![LookupCall {
            model_id: String::from("m_embedded"),
            team_id: None,
        }]
    );
}

#[tokio::test]
async fn unified_file_routing_legacy_param_model_routes() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_body", r#"{"api_key":"sk-b"}"#)]);
    let mut probe = Probe::get();
    probe.headers = header_map(&[("x-litellm-model", "m_header")]);
    probe.query = Some("model=m_query");
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"m_body"}"#);
    let outcome = unified_file_routing(&mut data, "file_raw", &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(String::from("file_raw")));
    assert_eq!(data, object(r#"{"model":"m_body","api_key":"sk-b"}"#));
    assert_eq!(grants.asked(), vec!["m_body"]);
}

#[tokio::test]
async fn unified_file_routing_legacy_query_when_body_absent() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_last", r#"{"api_key":"sk-l"}"#)]);
    let mut probe = Probe::get();
    probe.query = Some("model=m_query&model=m_last");
    probe.headers = header_map(&[("x-litellm-model", "m_header")]);
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    let outcome = unified_file_routing(&mut data, "file_raw", &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(String::from("file_raw")));
    assert_eq!(data, object(r#"{"api_key":"sk-l"}"#));
    assert_eq!(grants.asked(), vec!["m_last"]);
}

#[tokio::test]
async fn unified_file_routing_legacy_header_when_others_absent() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_header", r#"{"api_key":"sk-h"}"#)]);
    let mut probe = Probe::get();
    probe.headers = header_map(&[("x-litellm-model", "m_header")]);
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    let outcome = unified_file_routing(&mut data, "file_raw", &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), Some(String::from("file_raw")));
    assert_eq!(data, object(r#"{"api_key":"sk-h"}"#));
    assert_eq!(grants.asked(), vec!["m_header"]);
}

#[tokio::test]
async fn unified_file_routing_legacy_unrecognized_returns_none() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m1", r#"{"api_key":"sk-m1"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"file_id":"file_raw"}"#);
    let outcome = unified_file_routing(&mut data, "file_raw", &facts, &caller, &deps).await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(data, object(r#"{"file_id":"file_raw"}"#));
    assert!(catalog.lookups().is_empty());
}

#[tokio::test]
async fn unified_file_routing_legacy_authorization_denied() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::denying("m_body");
    let catalog = Catalog::with(&[("m_body", r#"{"api_key":"sk-b"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"m_body"}"#);
    let outcome = unified_file_routing(&mut data, "file_raw", &facts, &caller, &deps).await;
    assert_eq!(
        authorization_failure(outcome).to_string(),
        "the scripted grant denied `m_body`"
    );
    assert!(catalog.lookups().is_empty());
}

#[tokio::test]
async fn unified_file_routing_legacy_lookup_missing_model_errors() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::default();
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"m_missing"}"#);
    let outcome = unified_file_routing(&mut data, "file_raw", &facts, &caller, &deps).await;
    let (status, detail) = lookup_failure(outcome);
    assert_eq!(status, Some(400));
    assert!(
        detail
            .unwrap()
            .contains("Invalid model name passed in model=m_missing")
    );
}
