use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_context_sources::claude::RouterNames as _;
use betterlite_auth::auth_context_sources::router_settings::TeamRouterSettings;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_key_lookup::{KeyAuthenticator, KeyLookupOutcome};
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_keys::http::{FetchError, HttpReply, JwksHttp};
use betterlite_auth::jwt_keys::resolver::JwksResolver;
use betterlite_auth::jwt_keys::settings::JwtKeySettings;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::request_body::RequestBodyInput;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::ModelAccessObject;
use betterlite_http::error::HttpError;
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
    let app = routes::router(
        state_for(&model_config()),
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
    request.extensions_mut().insert(Arc::new(alias_rule()));
    request
        .extensions_mut()
        .insert(Arc::new(pattern_rule(allowed)));
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), status);
    if status == StatusCode::OK {
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
            br#"{"model":"group-one","user":"kept"}"#
        );
    }
}

const VIRTUAL_KEY: &str = "sk-virtual-key";

struct AcceptingKeys {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl KeyAuthenticator for AcceptingKeys {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        assert_eq!(presented, VIRTUAL_KEY);
        self.calls.fetch_add(1, Ordering::AcqRel);
        let fields = serde_json::from_str(
            r#"{"key_name":"key-name","user_id":"live-user","team_id":"live-team"}"#,
        )
        .unwrap();
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
        Ok(KeyLookupOutcome {
            auth,
            hashed_token: "hashed-virtual-key".to_owned(),
        })
    }
}

// The key lookup runs in prepare, before the alias rewrite, and the stage loop forwards its identity instead of reading the key again.
#[tokio::test]
async fn virtual_key_on_llm_route_authenticates_before_rewrite() {
    let calls = Arc::new(AtomicUsize::new(0));
    let authenticator: Arc<dyn KeyAuthenticator> = Arc::new(AcceptingKeys {
        calls: Arc::clone(&calls),
    });
    let app = routes::router(
        state_for(&model_config()),
        Arc::new(AtomicBool::new(false)),
        Router::new().route("/v1/chat/completions", post(echo)),
        Router::new(),
    )
    .layer(Extension(authenticator));
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {VIRTUAL_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"model":"alias-one","user":"kept"}"#))
        .unwrap();
    request.extensions_mut().insert(Arc::new(alias_rule()));
    request
        .extensions_mut()
        .insert(Arc::new(pattern_rule("group-one")));
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
        br#"{"model":"group-one","user":"kept"}"#
    );
}

const SIGNED_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImsxIn0.eyJzdWIiOiJ1c2VyLTEifQ.c2ln";

struct NoKeys;

#[async_trait]
impl KeyCache for NoKeys {
    async fn read_value(&self, _key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(None)
    }

    async fn write_value(
        &self,
        _key: &str,
        _value: &Json,
        _ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }
}

struct UnreachableKeys;

#[async_trait]
impl JwksHttp for UnreachableKeys {
    async fn get(&self, _url: &str) -> Result<HttpReply, FetchError> {
        Err(FetchError::Transport(HttpError::Timeout))
    }
}

struct FrozenClock;

#[async_trait]
impl Clock for FrozenClock {
    fn now(&self) -> f64 {
        1_700_000_000.0
    }

    async fn sleep(&self, _delay: Duration) {}
}

fn unreachable_resolver() -> Arc<JwksResolver> {
    let settings = JwtKeySettings::new(
        Some("https://idp.example.com/jwks".to_owned()),
        600.0,
        3_600.0,
        None,
        None,
    )
    .unwrap();
    Arc::new(JwksResolver::new(
        Arc::new(settings),
        Arc::new(NoKeys),
        Arc::new(UnreachableKeys),
        Arc::new(FrozenClock),
    ))
}

// prepare fetches the JWT signing key before its key lookup, so an unavailable IdP answers 503 instead of the lookup's 400.
#[tokio::test]
async fn unavailable_signing_key_answers_before_the_early_lookup() {
    let app = routes::router(
        state_for(&model_config()),
        Arc::new(AtomicBool::new(false)),
        Router::new().route("/v1/chat/completions", post(echo)),
        Router::new(),
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {SIGNED_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .extension(unreachable_resolver())
        .body(Body::from(r#"{"model":"group-one"}"#))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("auth_provider_unavailable"), "{text}");
}

fn model_config() -> Config {
    ConfigLoader { lookup: BTreeMap::<String, String>::new() }
        .load("general_settings:\n  master_key: secret-master-key\nmodel_list:\n  - model_name: group-one\n    litellm_params:\n      model: openai/group-one\n").unwrap()
}

fn pattern_rule(allowed: &str) -> ModelPatternRule {
    ModelPatternRule {
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
