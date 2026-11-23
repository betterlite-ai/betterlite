#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use chrono::{Duration, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{
    LifecycleState, UnbuiltJudgeSdk, identity_of_caller, lifecycle_response, start_route,
    stop_route, store_from_state,
};
use crate::auth::auth_identity::AuthContext;
use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::sdk::JudgeSdk as _;
use crate::routes::tests::state_for;
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_lifecycle::body::ShadowBodyError;
use crate::shadow_eval_lifecycle::error::LifecycleError;
use crate::shadow_eval_lifecycle::response::job_from_legs;
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_validation::error::AccessError;
use crate::shadow_result_aggregation::legs::LegRow;

fn principal(user_id: &str, roles: &[Role]) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: user_id.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: Some(UserIdentity {
            id: user_id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: roles.to_vec(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

fn caller(user_id: &str, roles: &[Role]) -> Extension<AuthContext> {
    Extension(AuthContext {
        principal: principal(user_id, roles),
    })
}

fn state() -> Arc<LifecycleState> {
    Arc::new(LifecycleState {
        store: None,
        api_key: None,
        api_base: None,
    })
}

fn judge_call() -> JudgeCall {
    JudgeCall {
        model: "judge-model".to_owned(),
        messages: Vec::new(),
        num_retries: 0,
        fallbacks: None,
        drop_params: false,
        params: IndexMap::new(),
    }
}

fn configured_config() -> Config {
    let yaml = "general_settings:\n  database_url: postgresql://betterlite:betterlite@localhost:5432/betterlite\n";
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(yaml)
    .unwrap()
}

#[tokio::test]
async fn an_unauthenticated_start_answers_401() {
    let response = start_route(State(state()), None, Bytes::new()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_non_admin_stop_answers_403() {
    let response = stop_route(
        State(state()),
        Path("job-1".to_owned()),
        Some(caller("user-1", &[])),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_admin_start_without_a_database_answers_500() {
    let response = start_route(
        State(state()),
        Some(caller("admin-1", &[Role::PlatformAdmin])),
        Bytes::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn identity_of_caller_refuses_a_request_without_a_principal() {
    let response = identity_of_caller(None).err().unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn identity_of_caller_reads_the_principal_the_auth_layer_published() {
    let published = caller("admin-1", &[Role::PlatformAdmin]);
    let identity = identity_of_caller(Some(&published)).unwrap();
    assert_eq!(identity.user_id.as_deref(), Some("admin-1"));
    assert_eq!(identity.user_role.as_deref(), Some("proxy_admin"));
}

#[test]
fn lifecycle_response_maps_every_error_code() {
    let too_large = LifecycleError::Body(ShadowBodyError::TooLarge { bytes: 2, limit: 1 });
    let forbidden = LifecycleError::Access(AccessError::WriterForbidden {
        action: "start a shadow eval".to_owned(),
    });
    let missing = LifecycleError::JobNotFound {
        job_id: "job-1".to_owned(),
    };
    let claimed = LifecycleError::AlreadyClaimed {
        direction: "forward".to_owned(),
        targets: "key key-1 (job job-1)".to_owned(),
    };
    let bad_request = lifecycle_response(Err(too_large), StatusCode::CREATED).status();
    let not_allowed = lifecycle_response(Err(forbidden), StatusCode::CREATED).status();
    let not_found = lifecycle_response(Err(missing), StatusCode::CREATED).status();
    let conflict = lifecycle_response(Err(claimed), StatusCode::CREATED).status();
    let unavailable = lifecycle_response(
        Err(LifecycleError::DatabaseUnavailable),
        StatusCode::CREATED,
    )
    .status();
    assert_eq!(bad_request, StatusCode::BAD_REQUEST);
    assert_eq!(not_allowed, StatusCode::FORBIDDEN);
    assert_eq!(not_found, StatusCode::NOT_FOUND);
    assert_eq!(conflict, StatusCode::CONFLICT);
    assert_eq!(unavailable, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn the_unbuilt_judge_sdk_refuses_a_call() {
    let error = UnbuiltJudgeSdk
        .acompletion(judge_call())
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.message,
        "start-time validation never dispatches a judge call"
    );
}

#[tokio::test]
async fn store_from_state_builds_a_store_from_the_configured_url() {
    let state = state_for(&configured_config());
    assert!(store_from_state(&state).is_some());
}

#[test]
fn lifecycle_response_passes_a_started_job_through_with_its_status() {
    let now = Utc::now();
    let leg = LegRow {
        id: "leg-1".to_owned(),
        group_id: "job-1".to_owned(),
        target_type: ShadowEvalTargetType::Key,
        target_id: "key-1".to_owned(),
        router_name: "shadow-router".to_owned(),
        router_names: vec!["shadow-router".to_owned()],
        models: Vec::new(),
        direction: ShadowEvalDirection::Forward,
        baseline_model: None,
        judge_model: "judge-model".to_owned(),
        shadow_percentage: 10.0_f64,
        max_turns: 10_000,
        max_budget: Some(Decimal::new(1000, 2)),
        created_at: now,
        ends_at: now + Duration::days(7),
        stopped_at: None,
        stopped_by: None,
    };
    let started = job_from_legs("job-1", &[leg], None)
        .unwrap()
        .response(now)
        .unwrap();
    let answered = lifecycle_response(Ok(started), StatusCode::CREATED);
    assert_eq!(answered.status(), StatusCode::CREATED);
}
