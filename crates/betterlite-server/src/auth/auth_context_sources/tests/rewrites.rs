use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_context_sources::claude::RouterNames as _;
use betterlite_auth::auth_context_sources::router_settings::TeamRouterSettings;
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::request_body::RequestBodyInput;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::ModelAccessObject;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use tower::ServiceExt as _;

use crate::auth::auth_context_sources::rewrite::{Markers, RewriteRule};
use crate::auth::auth_context_sources::selection::{Selection, select_model};
use crate::auth::auth_model_patterns::ModelPatternRule;
use crate::routes;
use crate::routes::tests::state_for;

#[test]
fn configured_names_and_sso() {
    let config = ConfigLoader { lookup: BTreeMap::from([("GOOGLE_CLIENT_ID".to_owned(), "client".to_owned())]) }
        .load("router_settings:\n  model_group_alias:\n    alias: group\nmodel_list:\n  - model_name: group\n    model_info:\n      id: deployment\n    litellm_params:\n      model: openai/group\n  - model_name: other\n    litellm_params:\n      model: openai/other\n").unwrap();
    let state = state_for(&config);
    let slot = &state.auth_context_sources;
    assert_eq!(slot.sso.google_client_id.as_deref(), Some("client"));
    assert!(slot.model_group_alias_contains("alias"));
    assert!(!slot.model_group_alias_contains("missing"));
    assert!(slot.has_model_id("deployment"));
    assert!(!slot.has_model_id("group"));
    assert_eq!(
        slot.candidate_model_ids_for_route("group", None),
        ["deployment"]
    );
    assert_eq!(slot.candidate_model_ids_for_route("other", None), ["other"]);
    assert!(
        slot.candidate_model_ids_for_route("missing", None)
            .is_empty()
    );
}

struct Teams;

#[async_trait::async_trait]
impl TeamRouterSettings for Teams {
    async fn router_settings(
        &self,
        team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError> {
        assert_eq!(team_id, "team-one");
        Ok(Some(BTreeMap::from([(
            "answer".to_owned(),
            JsonData::Bool(true),
        )])))
    }
}

#[tokio::test]
async fn team_source_is_forwarded() {
    let mut rule = RewriteRule::default();
    assert_eq!(rule.router_settings("team-one").await.unwrap(), None);
    rule.teams = Some(Arc::new(Teams));
    assert_eq!(
        rule.router_settings("team-one").await.unwrap(),
        Some(BTreeMap::from([(
            "answer".to_owned(),
            JsonData::Bool(true)
        )]))
    );
}

#[rstest]
#[case::custom(Some(true), BTreeSet::new())]
#[case::provider(None, BTreeSet::from(["endpoint".to_owned()]))]
#[tokio::test]
async fn passthrough_selection_keeps_model(
    #[case] endpoint_marker: Option<bool>,
    #[case] path_params: BTreeSet<String>,
) {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .unwrap();
    let state = state_for(&config);
    let mut rule = alias_rule();
    rule.endpoint_marker = endpoint_marker;
    rule.path_params = path_params;
    let mut markers = Markers::default();
    let body = serde_json::from_str(r#"{"model":"alias-one"}"#).unwrap();
    let empty = JsonObject::new();
    let target = select_model(
        &state,
        &rule,
        &mut markers,
        Selection {
            route: "/v1/chat/completions",
            body: &body,
            headers: &empty,
            query: &empty,
            raw_available: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(target, None);
    assert!(markers.resolved);
    assert!(!markers.normalized);
    assert_eq!(markers.client_model, None);
}

async fn echo(request: Request<Body>) -> String {
    let cached = request.extensions().get::<RequestBodyInput>().unwrap();
    let visible = cached.cached_body.as_ref().unwrap().visible();
    let decoded = serde_json::to_string(&visible).unwrap();
    let raw = to_bytes(request.into_body(), 4096).await.unwrap();
    assert_eq!(raw.as_ref(), decoded.as_bytes());
    decoded
}

#[rstest]
#[case::unchanged_model("group-one", "group-one", StatusCode::OK)]
#[case::unchanged_model_still_needs_permission("group-one", "alias-one", StatusCode::FORBIDDEN)]
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
    let rule = alias_rule();
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

fn alias_rule() -> RewriteRule {
    RewriteRule {
        key_aliases: None,
        team_aliases: None,
        key_settings: Some(
            serde_json::from_str::<Json>(r#"{"model_group_alias":{"alias-one":"group-one"}}"#)
                .unwrap(),
        ),
        team_id: None,
        teams: None,
        endpoint_marker: None,
        path_params: BTreeSet::new(),
    }
}
