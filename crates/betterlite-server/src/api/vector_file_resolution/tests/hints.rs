use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;

use crate::api::vector_file_resolution::Caller;
use crate::api::vector_file_resolution::model_hint::{ModelHintStep, model_hint_precedence};
use crate::api::vector_file_resolution::team_fallback::{candidate_names, team_model_fallback};

use super::support::{
    Catalog, FILE_PATH, Grants, LookupCall, Probe, auth, authorization_failure, header_map,
    lookup_failure, object, principal, routing_deps, team_auth, texts,
};

#[tokio::test]
async fn model_hint_bypass_on_api_key_or_api_base() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m", r#"{"api_key":"sk"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    for text in [
        r#"{"model":"m","api_key":""}"#,
        r#"{"model":"m","api_base":""}"#,
        r#"{"model":"m","api_key":0}"#,
        r#"{"model":"m","api_base":false}"#,
        r#"{"model":"m","api_key":[]}"#,
    ] {
        let mut bypass_data = object(text);
        let step = model_hint_precedence(&mut bypass_data, &facts, Some(&caller), &deps)
            .await
            .unwrap();
        assert_eq!(step, ModelHintStep::Bypassed, "{text}");
    }
    assert!(catalog.lookups().is_empty());
    assert!(grants.asked().is_empty());
    let mut applied_data = object(r#"{"model":"m","api_base":null,"api_key":null}"#);
    let applied = model_hint_precedence(&mut applied_data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(applied, ModelHintStep::CredentialsApplied);
    assert_eq!(
        applied_data,
        object(r#"{"model":"m","api_base":null,"api_key":"sk"}"#)
    );
}

#[tokio::test]
async fn model_hint_body_wins_without_authorizing() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_body", r#"{"api_key":"sk-b"}"#)]);
    let mut probe = Probe::get();
    probe.query = Some("model=m_query");
    probe.headers = header_map(&[("x-litellm-model", "m_header")]);
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"m_body"}"#);
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::CredentialsApplied);
    assert_eq!(data, object(r#"{"model":"m_body","api_key":"sk-b"}"#));
    assert!(grants.asked().is_empty());
    assert_eq!(
        catalog.lookups(),
        vec![LookupCall {
            model_id: String::from("m_body"),
            team_id: None,
        }]
    );
}

#[tokio::test]
async fn model_hint_query_equals_body_authorizes() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_same", r#"{"api_key":"sk-s"}"#)]);
    let mut probe = Probe::get();
    probe.query = Some("model=m_same");
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"m_same"}"#);
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::CredentialsApplied);
    assert_eq!(grants.asked(), vec!["m_same"]);
}

#[tokio::test]
async fn model_hint_query_before_header() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[
        ("m_query", r#"{"api_key":"sk-q"}"#),
        ("m_header", r#"{"api_key":"sk-h"}"#),
    ]);
    let mut probe = Probe::get();
    probe.query = Some("model=m_query");
    probe.headers = header_map(&[("x-litellm-model", "m_header")]);
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::CredentialsApplied);
    assert_eq!(data, object(r#"{"api_key":"sk-q"}"#));
    assert_eq!(grants.asked(), vec!["m_query"]);
}

#[tokio::test]
async fn model_hint_empty_query_falls_to_header() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_header", r#"{"api_key":"sk-h"}"#)]);
    let mut probe = Probe::get();
    probe.query = Some("model=");
    probe.headers = header_map(&[("x-litellm-model", "m_header")]);
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":""}"#);
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::CredentialsApplied);
    assert_eq!(data, object(r#"{"model":"","api_key":"sk-h"}"#));
    assert_eq!(grants.asked(), vec!["m_header"]);
}

#[tokio::test]
async fn model_hint_wildcard_uses_team_lookup() {
    let principal = principal();
    let auth = team_auth(texts(&[]), Some("team-a"));
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("team/*", r#"{"api_key":"sk-t"}"#)]);
    let mut probe = Probe::get();
    probe.query = Some("model=team%2F%2A");
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"team/*"}"#);
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::CredentialsApplied);
    assert_eq!(grants.asked(), vec!["team/*"]);
    assert_eq!(
        catalog.lookups(),
        vec![LookupCall {
            model_id: String::from("team/*"),
            team_id: Some(String::from("team-a")),
        }]
    );
    assert_eq!(data, object(r#"{"model":"team/*","api_key":"sk-t"}"#));
}

#[tokio::test]
async fn model_hint_wildcard_without_router_falls_back() {
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
    let mut data = object(r#"{"model":"team/*"}"#);
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::FallbackRequired);
    assert_eq!(data, object(r#"{"model":"team/*"}"#));
}

#[tokio::test]
async fn model_hint_wildcard_miss_falls_back() {
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
    let mut data = object(r#"{"model":"team/*"}"#);
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::FallbackRequired);
    assert_eq!(catalog.lookups().len(), 1);
    assert!(grants.asked().is_empty());
}

#[tokio::test]
async fn model_hint_exact_without_router_errors() {
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
    let mut data = object(r#"{"model":"m"}"#);
    let outcome = model_hint_precedence(&mut data, &facts, Some(&caller), &deps).await;
    let (status, detail) = lookup_failure(outcome);
    assert_eq!(status, Some(500));
    assert_eq!(
        detail.as_deref(),
        Some("Router not initialized. Cannot use model-based routing.")
    );
}

#[tokio::test]
async fn model_hint_exact_missing_model_errors() {
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
    let outcome = model_hint_precedence(&mut data, &facts, Some(&caller), &deps).await;
    let (status, detail) = lookup_failure(outcome);
    assert_eq!(status, Some(400));
    assert!(
        detail
            .unwrap()
            .contains("Invalid model name passed in model=m_missing")
    );
}

#[tokio::test]
async fn model_hint_truthy_non_text_body_falls_back() {
    let principal = principal();
    let auth = auth(&[]);
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m", r#"{"api_key":"sk"}"#)]);
    let probe = Probe::get();
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":5,"api_key":null}"#);
    let step = model_hint_precedence(&mut data, &facts, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::FallbackRequired);
    assert!(catalog.lookups().is_empty());
}

#[tokio::test]
async fn model_hint_null_caller_skips_authorization() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m_same", r#"{"api_key":"sk-s"}"#)]);
    let mut probe = Probe::get();
    probe.query = Some("model=m_same");
    let facts = probe.facts(FILE_PATH);
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"m_same"}"#);
    let step = model_hint_precedence(&mut data, &facts, None, &deps)
        .await
        .unwrap();
    assert_eq!(step, ModelHintStep::CredentialsApplied);
    assert!(grants.asked().is_empty());
    assert_eq!(data, object(r#"{"model":"m_same","api_key":"sk-s"}"#));
}

#[tokio::test]
async fn team_fallback_without_caller_or_router() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[(
        "m",
        r#"{"custom_llm_provider":"openai","api_key":"sk-team"}"#,
    )]);
    let auth = team_auth(texts(&["m"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    team_model_fallback(&mut data, None, &deps).await.unwrap();
    assert!(catalog.lookups().is_empty());
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"api_key":"sk-team"}"#));
    let loose_deps = routing_deps(None, &grants, None, false);
    team_model_fallback(&mut data, Some(&caller), &loose_deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"api_key":"sk-team"}"#));
    assert_eq!(catalog.lookups().len(), 1);
    assert_eq!(grants.asked(), vec!["m"]);
}

#[tokio::test]
async fn team_fallback_one_openai_deployment_applies() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[(
        "m",
        r#"{"custom_llm_provider":"openai","api_key":"sk-team"}"#,
    )]);
    let auth = team_auth(texts(&["m"]), Some("team-a"));
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"request"}"#);
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"model":"request","api_key":"sk-team"}"#));
    assert_eq!(grants.asked(), vec!["m"]);
    assert_eq!(
        catalog.lookups(),
        vec![LookupCall {
            model_id: String::from("m"),
            team_id: Some(String::from("team-a")),
        }]
    );
}

#[tokio::test]
async fn team_fallback_infers_openai_from_model() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m", r#"{"model":"openai/gpt-4o","api_key":"sk-i"}"#)]);
    let auth = team_auth(texts(&["m"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(
        data,
        object(r#"{"model":"openai/gpt-4o","api_key":"sk-i"}"#)
    );
    assert_eq!(grants.asked(), vec!["m"]);
}

#[tokio::test]
async fn team_fallback_infers_when_provider_null() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[(
        "m",
        r#"{"custom_llm_provider":null,"model":"openai/gpt-4o","api_key":"sk-i"}"#,
    )]);
    let auth = team_auth(texts(&["m"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(
        data,
        object(r#"{"model":"openai/gpt-4o","api_key":"sk-i"}"#)
    );
    assert_eq!(grants.asked(), vec!["m"]);
}

#[tokio::test]
async fn team_fallback_skips_non_openai_provider() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[(
        "m",
        r#"{"custom_llm_provider":"azure","model":"openai/gpt-4o","api_key":"sk-i"}"#,
    )]);
    let auth = team_auth(texts(&["m"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"model":"m"}"#));
    assert_eq!(grants.asked(), vec!["m"]);
}

#[tokio::test]
async fn team_fallback_two_openai_matches_authorize_both() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[
        ("a", r#"{"custom_llm_provider":"openai","api_key":"sk-a"}"#),
        ("b", r#"{"custom_llm_provider":"openai","api_key":"sk-b"}"#),
    ]);
    let auth = team_auth(texts(&["a", "b"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"request"}"#);
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"model":"request"}"#));
    assert_eq!(grants.asked(), vec!["a", "b"]);
    assert_eq!(catalog.lookups().len(), 2);
}

#[tokio::test]
async fn team_fallback_duplicate_names_count_twice() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[(
        "dup",
        r#"{"custom_llm_provider":"openai","api_key":"sk-d"}"#,
    )]);
    let auth = team_auth(texts(&["dup", "dup"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"request"}"#);
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"model":"request"}"#));
    assert_eq!(grants.asked(), vec!["dup", "dup"]);
    assert_eq!(catalog.lookups().len(), 2);
}

#[tokio::test]
async fn team_fallback_single_remaining_name_sets_model() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[(
        "only",
        r#"{"custom_llm_provider":"azure","api_key":"sk-a"}"#,
    )]);
    let auth = team_auth(texts(&["only"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"request"}"#);
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"model":"only"}"#));
    assert_eq!(grants.asked(), vec!["only"]);
}

#[tokio::test]
async fn team_fallback_no_team_models_leaves_data() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m", r#"{"custom_llm_provider":"openai","api_key":"sk"}"#)]);
    let auth = team_auth(texts(&[]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"request"}"#);
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"model":"request"}"#));
    assert!(grants.asked().is_empty());
    assert!(catalog.lookups().is_empty());
}

#[tokio::test]
async fn team_fallback_multiple_non_openai_leaves_data() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[
        ("a", r#"{"custom_llm_provider":"azure"}"#),
        ("b", r#"{"custom_llm_provider":"azure"}"#),
    ]);
    let auth = team_auth(texts(&["a", "b"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"request"}"#);
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"model":"request"}"#));
    assert!(grants.asked().is_empty());
    assert_eq!(catalog.lookups().len(), 2);
}

#[tokio::test]
async fn team_fallback_skips_sentinels_and_non_strings() {
    let grants = Grants::default();
    let catalog = Catalog::with(&[("m", r#"{"custom_llm_provider":"openai","api_key":"sk"}"#)]);
    let mixed = JsonData::Array(vec![
        JsonData::String(String::from("all-team-models")),
        JsonData::String(String::from("all-proxy-models")),
        JsonData::String(String::from("no-default-models")),
        JsonData::Bool(true),
        JsonData::String(String::from("m")),
    ]);
    let auth = team_auth(mixed, None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object("{}");
    team_model_fallback(&mut data, Some(&caller), &deps)
        .await
        .unwrap();
    assert_eq!(data, object(r#"{"api_key":"sk"}"#));
    assert_eq!(grants.asked(), vec!["m"]);
    assert_eq!(catalog.lookups().len(), 1);
}

#[tokio::test]
async fn team_fallback_authorization_denied() {
    let grants = Grants::denying("m");
    let catalog = Catalog::with(&[("m", r#"{"custom_llm_provider":"openai","api_key":"sk"}"#)]);
    let auth = team_auth(texts(&["m"]), None);
    let principal = principal();
    let caller = Caller {
        principal: &principal,
        auth: &auth,
    };
    let deps = routing_deps(Some(&catalog), &grants, None, false);
    let mut data = object(r#"{"model":"request"}"#);
    let outcome = team_model_fallback(&mut data, Some(&caller), &deps).await;
    assert_eq!(
        authorization_failure(outcome).to_string(),
        "the scripted grant denied `m`"
    );
    assert_eq!(data, object(r#"{"model":"request"}"#));
}

#[test]
fn team_fallback_candidate_names_shapes() {
    assert_eq!(candidate_names(None), Some(Vec::new()));
    let falsy = serde_json::from_str::<Json>("false").unwrap();
    assert_eq!(candidate_names(Some(&falsy)), Some(Vec::new()));
    let non_list = serde_json::from_str::<Json>(r#""all-team-models""#).unwrap();
    assert_eq!(candidate_names(Some(&non_list)), None);
    let list = serde_json::from_str::<Json>(r#"["a","all-team-models","",5,"b","a"]"#).unwrap();
    assert_eq!(
        candidate_names(Some(&list)),
        Some(vec![
            String::from("a"),
            String::new(),
            String::from("b"),
            String::from("a"),
        ])
    );
}
