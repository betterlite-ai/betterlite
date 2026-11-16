#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_model_lists::complete_list::{ListingFlags, ListingRequest};
use betterlite_config::loader::ConfigLoader;
use indexmap::IndexMap;
use tower::ServiceExt as _;

use crate::routes;
use crate::routes::tests::state_for;

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

fn config() -> betterlite_config::types::Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(CONFIG)
    .expect("the test config loads")
}

fn strings(models: &[&str]) -> Vec<String> {
    models.iter().map(|model| (*model).to_owned()).collect()
}

fn request<'request>(groups: &'request IndexMap<String, Vec<String>>) -> ListingRequest<'request> {
    ListingRequest {
        key_models: &[],
        team_models: &[],
        user_model: None,
        team_id: None,
        model_access_groups: groups,
        flags: ListingFlags::default(),
    }
}

#[test]
fn listing_service_is_registered() {
    let document = config();
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
    let state = state_for(&config());
    let groups = IndexMap::new();
    let authorized = state
        .auth_model_lists
        .authorized_models(&request(&groups))
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
    let body = String::from_utf8(body.to_vec()).expect("the body is UTF-8");
    assert_eq!(
        body, LISTED,
        "the model-list route lists the same names in the order the listing service returns"
    );
}
