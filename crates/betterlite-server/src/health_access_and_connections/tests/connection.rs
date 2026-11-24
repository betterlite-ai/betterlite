use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use rstest::rstest;

use crate::health_access_and_connections::connection_error::model_call_rejection;
use crate::health_access_and_connections::visibility::HealthRouter;
use crate::model_write_fields::error::ModelWriteError;

use super::support::{
    FakeProbe, FakeTeams, call, call_body, deployment, detail_error, failing_router, fake_router,
    key_context, master_context, object, single_seen, state, state_with_router,
    state_without_database, text,
};

const TEAM_BODY: &str = r#"{"litellm_params": {"model": "gpt-4o"}, "mode": "chat"}"#;
const MEMBER_KEY: &str =
    r#"{"user_id": "member-1", "user_role": "internal_user", "team_id": "t2"}"#;
const TEAMLESS_MEMBER_KEY: &str = r#"{"user_id": "member-1", "user_role": "internal_user"}"#;
const TEAM_ADMIN_KEY: &str =
    r#"{"user_id": "admin-1", "user_role": "internal_user", "team_id": "t1"}"#;
const VIEW_ONLY_KEY: &str = r#"{"user_id": "viewer-1", "user_role": "internal_user_viewer"}"#;
const DB_NOT_CONNECTED: &str = "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys";
const ENVIRONMENT: &str =
    "Environment variable references are not permitted in request parameters.";
const VIEW_ONLY: &str = "View-only users cannot manage models.";

fn config_deployment() -> DeploymentSpec {
    deployment(
        "gpt-4o",
        "d1",
        vec![
            ("model", text("openai/gpt-4o")),
            ("api_base", text("https://config")),
            ("api_key", text("sk-config")),
            ("custom_llm_provider", text("openai")),
        ],
        vec![("team_id", text("t1"))],
    )
}

fn gpt_router() -> Vec<DeploymentSpec> {
    vec![config_deployment()]
}

fn error_object(body: &JsonObject) -> &JsonObject {
    match body.get("error") {
        Some(Json::Object(error)) => error,
        _ => panic!("the body carries an error object"),
    }
}

fn result_of(body: &JsonObject) -> &JsonObject {
    match body.get("result") {
        Some(Json::Object(result)) => result,
        _ => panic!("the body carries a result object"),
    }
}

fn status_of(body: &JsonObject) -> &str {
    match body.get("status") {
        Some(Json::Text(status)) => status,
        _ => panic!("the body carries a status"),
    }
}

#[test]
fn a_rejection_renders_its_detail() {
    let view_only = model_call_rejection(&ModelWriteError::ViewOnly, None);
    assert_eq!(view_only.to_string(), VIEW_ONLY);
    let missing_error = ModelWriteError::TeamMissing {
        team_id: "t9".to_owned(),
    };
    let missing = model_call_rejection(&missing_error, None);
    assert_eq!(missing.to_string(), "Team id=t9 does not exist in db");
}

#[tokio::test]
async fn without_a_caller_the_route_answers_401_before_it_reads_the_body() {
    let probe = FakeProbe::answering(JsonObject::new());
    let reply = call(state(Vec::new(), &probe, FakeTeams::default()), None, "{").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let body = reply.json();
    let error = error_object(&body);
    assert_eq!(error.get("type").cloned(), Some(text("auth_error")));
    assert_eq!(error.get("param").cloned(), Some(text("None")));
    assert_eq!(error.get("code").cloned(), Some(text("401")));
    assert_eq!(
        error.get("message").cloned(),
        Some(text("the API key is not valid"))
    );
}

#[rstest]
#[case::truncated("{")]
#[case::trailing(r#"{"litellm_params": {}} {}"#)]
#[case::unknown_mode(r#"{"mode": "not-a-mode"}"#)]
#[case::wrong_params_type(r#"{"litellm_params": "model"}"#)]
#[tokio::test]
async fn a_body_outside_the_schema_is_a_422(#[case] body: &str) {
    let probe = FakeProbe::answering(JsonObject::new());
    let reply = call(
        state(Vec::new(), &probe, FakeTeams::default()),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let decoded = reply.json();
    assert!(matches!(decoded.get("detail"), Some(Json::List(_))));
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn without_a_database_the_route_answers_500() {
    let probe = FakeProbe::answering(JsonObject::new());
    let connection = state_without_database(&probe);
    let reply = call(connection, Some(&master_context()), TEAM_BODY).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(detail_error(&reply.json()), DB_NOT_CONNECTED);
    assert!(probe.seen().is_empty());
}

#[rstest]
#[case::top_level(r#"{"litellm_params": {"api_key": "os.environ/OPENAI_API_KEY"}}"#)]
#[case::nested(
    r#"{"litellm_params": {"vertex_credentials": {"api_key": "os.environ/OPENAI_API_KEY"}}}"#
)]
#[case::model_info(r#"{"litellm_params": {"model": "gpt-4o"}, "model_info": {"health_check_params": {"note": "os.environ/X"}}}"#)]
#[tokio::test]
async fn a_request_environment_reference_is_a_400_wherever_it_sits(#[case] body: &str) {
    let probe = FakeProbe::answering(JsonObject::new());
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(detail_error(&reply.json()), ENVIRONMENT);
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn a_request_url_is_401_and_never_reaches_the_probe() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "gpt-4o", "api_base": "https://elsewhere"}, "mode": "chat"}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let decoded = reply.json();
    let error = error_object(&decoded);
    assert_eq!(error.get("type").cloned(), Some(text("auth_error")));
    assert_eq!(error.get("param").cloned(), Some(text("None")));
    assert_eq!(error.get("code").cloned(), Some(text("401")));
    let Some(Json::Text(message)) = error.get("message") else {
        panic!("the error carries a message");
    };
    assert!(message.starts_with(
        "Authentication Error, Rejected Request: api_base is not allowed in request body."
    ));
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn the_client_credential_opt_in_still_refuses_urls() {
    let probe = FakeProbe::answering(JsonObject::new());
    let mut connection = state(gpt_router(), &probe, FakeTeams::holding(&["t1"]));
    connection.allow_client_side_credentials = true;
    let body = r#"{"litellm_params": {"model": "gpt-4o", "api_base": "https://elsewhere"}}"#;
    let reply = call(connection, Some(&master_context()), body).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(probe.seen().is_empty());

    let allowed = FakeProbe::answering(JsonObject::new());
    let mut opted_in = state(gpt_router(), &allowed, FakeTeams::holding(&["t1"]));
    opted_in.allow_client_side_credentials = true;
    let allowed_body =
        r#"{"litellm_params": {"model": "gpt-4o", "user_config": {"team": "a"}}, "mode": "chat"}"#;
    let allowed_reply = call(opted_in, Some(&master_context()), allowed_body).await;
    assert_eq!(allowed_reply.status, StatusCode::OK);
    let (params, _mode) = single_seen(&allowed);
    assert_eq!(
        params.get("user_config").cloned(),
        Some(Json::Object(object(vec![("team", text("a"))])))
    );
}

#[tokio::test]
async fn a_health_check_override_url_is_401_too() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "gpt-4o"}, "model_info": {"health_check_params": {"api_base": "https://elsewhere"}}, "mode": "chat"}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn a_configured_deployment_supplies_its_credentials() {
    let probe = FakeProbe::answering(object(vec![("x-ratelimit-remaining-requests", text("10"))]));
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(status_of(&body), "success");
    let (params, mode) = single_seen(&probe);
    assert_eq!(mode, Some("chat".to_owned()));
    assert_eq!(
        params.get("api_base").cloned(),
        Some(text("https://config"))
    );
    assert_eq!(params.get("api_key").cloned(), Some(text("sk-config")));
    assert_eq!(params.get("model").cloned(), Some(text("gpt-4o")));
    assert!(matches!(params.get("messages"), Some(Json::List(_))));
    let result = result_of(&body);
    assert_eq!(
        result.get("api_base").cloned(),
        Some(text("https://config"))
    );
    assert_eq!(
        result.get("x-ratelimit-remaining-requests").cloned(),
        Some(text("10"))
    );
    assert!(!result.contains_key("api_key"));
}

#[tokio::test]
async fn another_credential_name_strips_the_config_connections() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "gpt-4o", "litellm_credential_name": "other"}, "mode": "chat"}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let (params, _mode) = single_seen(&probe);
    assert!(!params.contains_key("api_base"));
    assert!(!params.contains_key("api_key"));
    assert_eq!(
        params.get("litellm_credential_name").cloned(),
        Some(text("other"))
    );
    assert_eq!(params.get("model").cloned(), Some(text("gpt-4o")));
}

#[tokio::test]
async fn an_id_selects_the_deployment_and_its_loaded_model_info_wins() {
    let deployments = vec![
        deployment(
            "gpt-4o",
            "d1",
            vec![
                ("model", text("openai/gpt-4o")),
                ("api_key", text("sk-one")),
            ],
            vec![("team_id", text("t1"))],
        ),
        deployment(
            "gpt-4o",
            "d2",
            vec![
                ("model", text("openai/gpt-4o")),
                ("api_key", text("sk-two")),
            ],
            vec![
                ("team_id", text("t1")),
                ("health_check_model", text("loaded-model")),
            ],
        ),
    ];
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "gpt-4o"}, "model_info": {"id": "d2", "health_check_model": "request-model"}, "mode": "chat"}"#;
    let reply = call(
        state(deployments, &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let (params, _mode) = single_seen(&probe);
    assert_eq!(params.get("api_key").cloned(), Some(text("sk-two")));
    assert_eq!(params.get("model").cloned(), Some(text("loaded-model")));
}

#[tokio::test]
async fn a_provider_model_string_reaches_the_deployment_serving_it() {
    let probe = FakeProbe::answering(JsonObject::new());
    let deployments = vec![deployment(
        "alias",
        "d1",
        vec![
            ("model", text("openai/special")),
            ("api_key", text("sk-alias")),
        ],
        Vec::new(),
    )];
    let body = r#"{"litellm_params": {"model": "openai/special"}, "mode": "chat"}"#;
    let reply = call(
        state(deployments, &probe, FakeTeams::default()),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let (params, _mode) = single_seen(&probe);
    assert_eq!(params.get("api_key").cloned(), Some(text("sk-alias")));
}

#[rstest]
#[case::no_router(false, false)]
#[case::router_error(false, true)]
#[case::router_miss(true, false)]
#[tokio::test]
async fn a_miss_or_a_router_failure_probes_the_request_params_only(
    #[case] with_router: bool,
    #[case] failing: bool,
) {
    let probe = FakeProbe::answering(JsonObject::new());
    let router: Option<Arc<dyn HealthRouter>> = if failing {
        Some(Arc::new(failing_router()))
    } else if with_router {
        let selected: Arc<dyn HealthRouter> = Arc::new(fake_router(gpt_router()));
        Some(selected)
    } else {
        None
    };
    let connection = state_with_router(router, &probe, FakeTeams::default());
    let body =
        r#"{"litellm_params": {"model": "unknown", "api_key": "sk-request"}, "mode": "chat"}"#;
    let reply = call(connection, Some(&master_context()), body).await;
    assert_eq!(reply.status, StatusCode::OK);
    let (params, mode) = single_seen(&probe);
    assert_eq!(mode, Some("chat".to_owned()));
    assert_eq!(params.get("api_key").cloned(), Some(text("sk-request")));
    assert!(!params.contains_key("api_base"));
}

#[tokio::test]
async fn a_view_only_caller_cannot_probe() {
    let probe = FakeProbe::answering(JsonObject::new());
    let caller = key_context(VIEW_ONLY_KEY);
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&caller),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(detail_error(&reply.json()), VIEW_ONLY);
    assert!(probe.seen().is_empty());
}

#[rstest]
#[case::another_team(
    MEMBER_KEY,
    "Team ID=t1 does not match the API key's team ID=t2, OR you are not the admin for this team. Check `/user/info` to verify your team admin status."
)]
#[case::no_team(
    TEAMLESS_MEMBER_KEY,
    "Team ID=t1 does not match the API key's team ID=None, OR you are not the admin for this team. Check `/user/info` to verify your team admin status."
)]
#[tokio::test]
async fn a_member_who_is_not_the_teams_admin_cannot_probe_it(
    #[case] key: &str,
    #[case] message: &str,
) {
    let probe = FakeProbe::answering(JsonObject::new());
    let caller = key_context(key);
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1", "t2"])),
        Some(&caller),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(detail_error(&reply.json()), message);
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn a_caller_without_a_team_cannot_probe_a_teamless_model() {
    let probe = FakeProbe::answering(JsonObject::new());
    let deployments = vec![deployment(
        "gpt-4o",
        "d1",
        vec![("model", text("openai/gpt-4o"))],
        Vec::new(),
    )];
    let caller = key_context(TEAMLESS_MEMBER_KEY);
    let reply = call(
        state(deployments, &probe, FakeTeams::default()),
        Some(&caller),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        detail_error(&reply.json()),
        "User does not have permission to make this model call. Your role=internal_user. You can only make model calls if you are a PROXY_ADMIN or if you are a team admin, by specifying a team_id in the model_info."
    );
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn a_team_admin_can_probe_its_teams_deployment() {
    let probe = FakeProbe::answering(JsonObject::new());
    let caller = key_context(TEAM_ADMIN_KEY);
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&caller),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(status_of(&reply.json()), "success");
    assert_eq!(probe.seen().len(), 1);
}

#[tokio::test]
async fn a_proxy_admin_can_probe_a_teams_deployment() {
    let probe = FakeProbe::answering(JsonObject::new());
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(probe.seen().len(), 1);
}

#[tokio::test]
async fn a_team_id_with_no_team_is_a_400() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "unknown"}, "model_info": {"team_id": "missing"}, "mode": "chat"}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        detail_error(&reply.json()),
        "Team id=missing does not exist in db"
    );
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn a_request_team_id_authorizes_against_that_team() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "unknown"}, "model_info": {"team_id": "t1"}, "mode": "chat"}"#;
    let caller = key_context(TEAM_ADMIN_KEY);
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&caller),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(probe.seen().len(), 1);
}

#[tokio::test]
async fn a_team_id_of_the_wrong_type_is_a_500() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body =
        r#"{"litellm_params": {"model": "unknown"}, "model_info": {"team_id": 5}, "mode": "chat"}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail_error(&reply.json()),
        "Failed to test connection: the model_info team_id must be a string"
    );
    assert!(probe.seen().is_empty());
}

#[tokio::test]
async fn the_body_mode_wins_over_the_params_mode_and_the_params_key_stays() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "gpt-4o", "mode": "chat"}, "mode": "embedding"}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let (params, mode) = single_seen(&probe);
    assert_eq!(mode, Some("embedding".to_owned()));
    assert_eq!(params.get("mode").cloned(), Some(text("chat")));
}

#[tokio::test]
async fn the_params_mode_is_taken_out_when_the_body_names_none() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "gpt-4o", "mode": "embedding"}}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let (params, mode) = single_seen(&probe);
    assert_eq!(mode, Some("embedding".to_owned()));
    assert!(!params.contains_key("mode"));
}

#[tokio::test]
async fn a_non_text_params_mode_is_probed_with_no_mode() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = r#"{"litellm_params": {"model": "gpt-4o", "mode": 5}}"#;
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let (params, mode) = single_seen(&probe);
    assert_eq!(mode, None);
    assert!(!params.contains_key("mode"));
}

#[rstest]
#[case::text_error(text("boom"))]
#[case::null_error(Json::Null)]
#[tokio::test]
async fn a_probe_result_carrying_an_error_is_a_200_with_status_error(#[case] error: Json) {
    let probe = FakeProbe::answering(object(vec![("error", error)]));
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(status_of(&body), "error");
    assert!(result_of(&body).contains_key("error"));
}

#[tokio::test]
async fn the_result_keeps_only_the_listed_display_fields() {
    let probe = FakeProbe::answering(object(vec![
        ("x-ms-region", text("eastus")),
        ("hidden", text("x")),
    ]));
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(status_of(&body), "success");
    let result = result_of(&body);
    assert_eq!(result.get("x-ms-region").cloned(), Some(text("eastus")));
    assert!(!result.contains_key("hidden"));
    assert!(!result.contains_key("api_key"));
}

#[tokio::test]
async fn a_probe_failure_is_a_500() {
    let probe = FakeProbe::failing("boom");
    let reply = call(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        TEAM_BODY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail_error(&reply.json()),
        "Failed to test connection: boom"
    );
}

#[tokio::test]
async fn an_oversized_body_is_a_413() {
    let probe = FakeProbe::answering(JsonObject::new());
    let body = Body::from(vec![b' '; 16 * 1024 * 1024 + 1]);
    let reply = call_body(
        state(gpt_router(), &probe, FakeTeams::holding(&["t1"])),
        Some(&master_context()),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);
    let decoded = reply.json();
    let error = error_object(&decoded);
    assert_eq!(
        error.get("type").cloned(),
        Some(text("invalid_request_error"))
    );
    assert_eq!(error.get("param").cloned(), Some(text("body")));
    assert_eq!(error.get("code").cloned(), Some(text("request_too_large")));
    assert!(probe.seen().is_empty());
}
