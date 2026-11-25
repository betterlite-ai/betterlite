use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_db::budget_permission_rows::JsonData;

use crate::key_limit_policy::checks::check_project_key_limits;
use crate::key_limit_policy::error::{KeyLimitError, Rejection};
use crate::key_limit_policy::request::KeyLimitRequest;

use super::support::{FakeReads, error_text, generate, project, rejected};

fn stored(models: &str, budget: &str) -> CachedProject {
    project(&format!(
        r#"{{"project_id": "proj-1", "models": {models}, "litellm_budget_table": {budget}}}"#
    ))
}

async fn check(reads: &FakeReads, request: &str) -> Result<(), KeyLimitError> {
    let parsed = generate(request);
    check_project_key_limits("proj-1", &KeyLimitRequest::from(&parsed), reads).await
}

#[tokio::test]
async fn project_budget_ceiling() {
    let reads = FakeReads::default().with_project(stored(
        "[]",
        r#"{"budget_id": "budget-1", "max_budget": 10}"#,
    ));
    let rejection = rejected(check(&reads, r#"{"max_budget": 11}"#).await);
    assert_eq!(
        error_text(&rejection),
        "Key max_budget (11) exceeds project's max_budget (10). Project: proj-1"
    );
    check(&reads, r#"{"max_budget": 10}"#).await.unwrap();
    let fractional_rejection = rejected(check(&reads, r#"{"max_budget": 10.5}"#).await);
    assert_eq!(
        error_text(&fractional_rejection),
        "Key max_budget (10.5) exceeds project's max_budget (10). Project: proj-1"
    );
}

#[tokio::test]
async fn project_budget_absent() {
    let mut reads = FakeReads::default().with_project(stored(
        "[]",
        r#"{"budget_id": "budget-1", "max_budget": null}"#,
    ));
    check(&reads, r#"{"max_budget": 1000}"#).await.unwrap();
    reads.project = Some(stored("[]", "null"));
    check(&reads, r#"{"max_budget": 1000}"#).await.unwrap();
    reads.project = Some(stored(
        "[]",
        r#"{"budget_id": "budget-1", "max_budget": 1}"#,
    ));
    check(&reads, r#"{"key_alias": "alias-1"}"#).await.unwrap();
}

#[tokio::test]
async fn project_missing() {
    let reads = FakeReads::default();
    let rejection = rejected(check(&reads, r#"{"max_budget": 1}"#).await);
    assert_eq!(
        error_text(&rejection),
        "Project not found, project_id=proj-1"
    );
    assert!(matches!(rejection, Rejection::ProjectNotFound { .. }));
}

#[tokio::test]
async fn project_empty_models() {
    let reads = FakeReads::default().with_project(stored("[]", "null"));
    check(&reads, r#"{"models": ["anything"]}"#).await.unwrap();
    check(&reads, r#"{"models": [42]}"#).await.unwrap();
}

#[tokio::test]
async fn project_model_subset() {
    let reads = FakeReads::default().with_project(stored(r#"["gpt-4"]"#, "null"));
    check(
        &reads,
        r#"{"models": ["gpt-4", "all-team-models", "all-proxy-models"]}"#,
    )
    .await
    .unwrap();
    let rejection = rejected(check(&reads, r#"{"models": ["gpt-4", "claude-4"]}"#).await);
    assert_eq!(
        error_text(&rejection),
        "Model \"claude-4\" not in project's allowed models. Project allowed models=[\"gpt-4\"]. Project: proj-1"
    );
}

#[tokio::test]
async fn project_model_order() {
    let reads = FakeReads::default().with_project(stored(r#"["gpt-4"]"#, "null"));
    let rejection = rejected(check(&reads, r#"{"models": ["z-model", "a-model"]}"#).await);
    match rejection {
        Rejection::ProjectModel { model, .. } => {
            assert_eq!(model, JsonData::String("z-model".to_owned()));
        }
        other @ (Rejection::NonFinite { .. }
        | Rejection::Upperbound { .. }
        | Rejection::Allocation { .. }
        | Rejection::ProjectNotFound { .. }
        | Rejection::ProjectBudget { .. }
        | Rejection::AssignWithoutUser
        | Rejection::NotMember { .. }) => {
            panic!("expected a project model rejection, got {other:?}")
        }
    }
    let non_string = rejected(check(&reads, r#"{"models": [42]}"#).await);
    assert!(matches!(non_string, Rejection::ProjectModel { .. }));
}

#[tokio::test]
async fn project_model_non_string_entry() {
    let reads = FakeReads::default().with_project(stored(r#"["gpt-4"]"#, "null"));
    let rejection = rejected(check(&reads, r#"{"models": [["gpt-4"]]}"#).await);
    assert!(matches!(rejection, Rejection::ProjectModel { .. }));
    let object_entry = rejected(check(&reads, r#"{"models": [{"name": "gpt-4"}]}"#).await);
    assert!(matches!(object_entry, Rejection::ProjectModel { .. }));
}

#[tokio::test]
async fn project_model_before_budget() {
    let reads = FakeReads::default().with_project(stored(
        r#"["gpt-4"]"#,
        r#"{"budget_id": "budget-1", "max_budget": 10}"#,
    ));
    let rejection = rejected(check(&reads, r#"{"models": ["claude-4"], "max_budget": 11}"#).await);
    assert!(matches!(rejection, Rejection::ProjectModel { .. }));
}
