#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::settings::Scope;

use crate::lens_access::error::failure_text;
use crate::lens_access::lookup::{get_lens, lens_repository, required};
use crate::lens_access::tests::support::{Harness, lens_in_team, lens_with, repository};

fn team_scope(team: &str) -> Scope {
    Scope {
        team_id: team.to_owned(),
        api_key_hash: String::new(),
        all_teams: false,
    }
}

fn all_teams_scope() -> Scope {
    Scope {
        team_id: String::new(),
        api_key_hash: String::new(),
        all_teams: true,
    }
}

#[tokio::test]
async fn a_missing_lens_is_not_found() {
    let harness = Harness::new();
    let error = get_lens(&repository(&harness), "missing", &all_teams_scope())
        .await
        .expect_err("a missing lens fails");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(failure_text(&error), "Lens not found");
}

#[tokio::test]
async fn an_accessible_lens_is_returned() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_in_team("l1", "team-1"));
    let lens = get_lens(&repository(&harness), "l1", &team_scope("team-1"))
        .await
        .expect("the team reads its lens");
    assert_eq!(lens.id, "l1");
}

#[tokio::test]
async fn another_teams_lens_is_not_found() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_in_team("l1", "team-1"));
    let error = get_lens(&repository(&harness), "l1", &team_scope("team-2"))
        .await
        .expect_err("another team's lens is hidden");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(failure_text(&error), "Lens not found");
}

#[tokio::test]
async fn a_team_scope_cannot_read_an_all_teams_lens() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_with("l1", true));
    let error = get_lens(&repository(&harness), "l1", &team_scope("team-1"))
        .await
        .expect_err("the all-teams lens is hidden");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[test]
fn required_turns_none_into_a_conflict() {
    let error = required(None).expect_err("no lens is a conflict");
    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        failure_text(&error),
        "Lens changed concurrently; retry the operation"
    );
}

#[test]
fn required_keeps_a_lens() {
    let lens = required(Some(lens_with("l1", true))).expect("the lens passes through");
    assert_eq!(lens.id, "l1");
}

#[test]
fn without_a_database_the_repository_helper_is_503() {
    let harness = Harness::new();
    let state = harness.state_without_database();
    let error = lens_repository(&state).err().expect("no repository");
    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        failure_text(&error),
        "Lens needs a connected Postgres database"
    );
}

#[test]
fn with_a_database_the_repository_helper_returns_it() {
    let harness = Harness::new();
    let state = harness.state();
    lens_repository(&state).expect("the repository is present");
}

#[tokio::test]
async fn a_repository_failure_is_reported_as_a_500() {
    let harness = Harness::new();
    harness.database.corrupted();
    let error = get_lens(&repository(&harness), "l1", &all_teams_scope())
        .await
        .expect_err("the corrupt record fails");
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!failure_text(&error).is_empty());
}
