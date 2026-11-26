use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::request_body::RequestBodyInput;
use betterlite_error::proxy_error_types::ModelAccessObject;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tower::ServiceExt as _;

use crate::auth::auth_context_sources::rewrite::RewriteRule;
use crate::auth::auth_model_patterns::ModelPatternRule;
use crate::routes;
use crate::routes::tests::state_for;

async fn echo(request: Request<Body>) -> String {
    let cached = request.extensions().get::<RequestBodyInput>().unwrap();
    let visible = cached.cached_body.as_ref().unwrap().visible();
    let decoded = serde_json::to_string(&visible).unwrap();
    let raw = to_bytes(request.into_body(), 4096).await.unwrap();
    assert_eq!(raw.as_ref(), decoded.as_bytes());
    decoded
}

#[rstest]
#[case::claude("group-one[1M]", "group-one", StatusCode::OK)]
#[case::router_alias("alias-one", "group-one", StatusCode::OK)]
#[case::alias_permission_is_not_target_permission("alias-one", "alias-one", StatusCode::FORBIDDEN)]
#[tokio::test]
async fn rewrites_before_authorization(
    #[case] requested: &str,
    #[case] allowed: &str,
    #[case] status: StatusCode,
) {
    let config = ConfigLoader { lookup: BTreeMap::<String, String>::new() }
        .load("general_settings:\n  master_key: secret-master-key\nmodel_list:\n  - model_name: group-one\n    litellm_params:\n      model: openai/group-one\n").unwrap();
    let app = routes::router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route("/v1/chat/completions", post(echo)),
        Router::new(),
    );
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            r#"{{"model":"{requested}","user":"kept"}}"#
        )))
        .unwrap();
    let mut rule = RewriteRule::default();
    rule.key_settings = Some(
        serde_json::from_str::<Json>(r#"{"model_group_alias":{"alias-one":"group-one"}}"#).unwrap(),
    );
    request.extensions_mut().insert(Arc::new(rule));
    request.extensions_mut().insert(Arc::new(ModelPatternRule {
        object: ModelAccessObject::Key,
        grant: ModelGrantPattern {
            models: vec![allowed.to_owned()],
            team_model_aliases: BTreeMap::new(),
            key_model_aliases: BTreeMap::new(),
            team_id: None,
        },
        view: None,
        provider: None,
        global_model_aliases: BTreeMap::new(),
    }));
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), status);
    if status == StatusCode::OK {
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
            br#"{"model":"group-one","user":"kept"}"#
        );
    }
}
