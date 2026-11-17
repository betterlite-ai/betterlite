#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_model_lists::access_groups::AccessGroupScope;
use betterlite_auth::auth_model_lists::complete_list::{ListingFlags, ListingRequest};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::provider_model_sets::ProviderModelSets;
use indexmap::IndexMap;
use tower::ServiceExt as _;

use crate::routes;
use crate::routes::tests::state_for;
use crate::state::auth_model_lists::Slot;
use crate::state::shared::Shared;

const CONFIG: &str = "model_list:
  - {model_name: oracle-shared, litellm_params: {model: openai/oracle-first}}
  - {model_name: oracle-azure, litellm_params: {model: azure/oracle-azure}}
  - {model_name: oracle-shared, litellm_params: {model: azure/oracle-second}}
  - {model_name: oracle-bare, litellm_params: {model: gpt-4o-mini}}
";

const LISTED: &str = concat!(
    r#"{"data":[{"id":"oracle-shared","object":"model","created":1677610602,"owned_by":"openai"},"#,
    r#"{"id":"oracle-azure","object":"model","created":1677610602,"owned_by":"openai"},"#,
    r#"{"id":"oracle-bare","object":"model","created":1677610602,"owned_by":"openai"}],"object":"list"}"#
);

const WILDCARD_CONFIG: &str = "model_list:
  - {model_name: wildcard-grant, litellm_params: {model: openai/oracle-first}}
environment_variables:
  OPENAI_API_KEY: test-openai-key
";

const ENDPOINT_CONFIG: &str = "model_list:
  - {model_name: endpoint-grant, litellm_params: {model: openai/oracle-first}}
litellm_settings:
  check_provider_endpoint: true
";

const REDIRECT_CONFIG: &str = "model_list:
  - {model_name: '*', litellm_params: {model: 'openai/*'}}
";

fn loaded(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the test config loads")
}

fn catalog_openai_models() -> Vec<String> {
    let mut sets = ProviderModelSets::default();
    let mut bundled = BundledCatalog::default();
    sets.add_known_models(&bundled.get());
    sets.models_by_provider
        .get("openai")
        .expect("the bundled catalog lists openai models")
        .iter()
        .cloned()
        .collect()
}

fn strings(models: &[&str]) -> Vec<String> {
    models.iter().map(|model| (*model).to_owned()).collect()
}

fn listing_request<'request>(
    key_models: &'request [String],
    groups: &'request IndexMap<String, Vec<String>>,
    flags: ListingFlags,
) -> ListingRequest<'request> {
    ListingRequest {
        key_models,
        team_models: &[],
        user_model: None,
        team_id: None,
        model_access_groups: groups,
        flags,
    }
}

#[test]
fn listing_service_is_registered() {
    let document = loaded(CONFIG);
    let state = state_for(&document);
    assert_eq!(
        state.auth_model_lists.service.proxy_model_list(),
        strings(&[
            "oracle-shared",
            "oracle-azure",
            "oracle-shared",
            "oracle-bare"
        ]),
        "the real state constructor installs the listing service from the config's model_list"
    );
}

#[tokio::test]
async fn listing_reads_authorized_models() {
    let state = state_for(&loaded(CONFIG));
    let groups = IndexMap::new();
    let authorized = state
        .auth_model_lists
        .authorized_models(&listing_request(&[], &groups, ListingFlags::default()))
        .expect("a wildcard-free config always lists");
    assert_eq!(
        authorized,
        ["oracle-shared", "oracle-azure", "oracle-bare"].map(str::to_owned)
    );
    let application = routes::router(
        Arc::clone(&state),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let response = application
        .oneshot(
            Request::get("/v1/models")
                .header("x-other", "2023-06-01")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    let payload = String::from_utf8(body.to_vec()).expect("the body is UTF-8");
    assert_eq!(
        payload, LISTED,
        "the model-list route lists the same names in the order the listing service returns"
    );
}

#[test]
fn listing_expands_catalog_wildcards() {
    let state = state_for(&loaded(WILDCARD_CONFIG));
    let groups = IndexMap::new();
    let catalog_models = catalog_openai_models();
    assert!(
        !catalog_models.is_empty(),
        "the bundled catalog lists openai models"
    );

    let provider_wildcard = strings(&["openai/*"]);
    let expanded = state
        .auth_model_lists
        .authorized_models(&listing_request(
            &provider_wildcard,
            &groups,
            ListingFlags::default(),
        ))
        .expect("a catalog provider wildcard always lists");
    let expected: Vec<String> = catalog_models
        .iter()
        .map(|model| {
            if model.starts_with("openai") {
                model.clone()
            } else {
                format!("openai/{model}")
            }
        })
        .collect();
    assert_eq!(
        expanded, expected,
        "the wildcard expands every model the catalog holds for its provider"
    );

    let bare_wildcard = strings(&["*"]);
    assert!(
        state
            .auth_model_lists
            .authorized_models(&listing_request(
                &bare_wildcard,
                &groups,
                ListingFlags::default()
            ))
            .expect("a bare wildcard without deployment rows lists nothing")
            .is_empty(),
        "a bare wildcard has no provider segment to expand"
    );

    let inferred = state
        .auth_model_lists
        .authorized_models(&listing_request(
            &[],
            &groups,
            ListingFlags {
                infer_model_from_keys: true,
                return_wildcard_routes: false,
                groups: AccessGroupScope::default(),
            },
        ))
        .expect("inference always lists");
    let mut inferred_expected = strings(&["wildcard-grant"]);
    inferred_expected.extend(catalog_models);
    assert_eq!(
        inferred, inferred_expected,
        "inference reads the provider list and the environment variable the config sets"
    );
}

#[test]
fn listing_provider_endpoint_check_lists_nothing() {
    let state = state_for(&loaded(ENDPOINT_CONFIG));
    let groups = IndexMap::new();
    let provider_wildcard = strings(&["openai/*"]);
    let expanded = state
        .auth_model_lists
        .authorized_models(&listing_request(
            &provider_wildcard,
            &groups,
            ListingFlags::default(),
        ))
        .expect("an endpoint-checked wildcard always lists");
    assert!(
        expanded.is_empty(),
        "no provider endpoint catalogue is built, so an endpoint check lists no models"
    );
}

#[test]
fn listing_has_no_deployment_rows() {
    let config = loaded(REDIRECT_CONFIG);
    let slot = Slot::new(
        &config,
        &Shared {
            bundled_catalog: BundledCatalog::default().get(),
        },
    );
    let groups = IndexMap::new();
    let granted_wildcard = strings(&["*"]);
    let expanded = slot
        .authorized_models(&listing_request(
            &granted_wildcard,
            &groups,
            ListingFlags::default(),
        ))
        .expect("a wildcard without deployment rows always lists");
    assert!(
        expanded.is_empty(),
        "the slot hands the listing no deployment rows, so a row's provider wildcard is never expanded"
    );
}
