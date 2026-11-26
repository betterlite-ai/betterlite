#![cfg(test)]

mod support;

use std::collections::BTreeMap;

use betterlite_config::sections::summary_model_access::{DEFAULT_SUMMARY_MAX_TOKENS, Settings};
use rstest::rstest;

use super::settings::read_summary_settings;
use support::{
    Call, EVERY_SCOPE, Harness, Lookup, Outcome, SUM, debug, every_scope_journal, journal_between,
    membership_row, principal, project_row, run, user_row, warn,
};

const MEMBERSHIP_FAILED: &str = "compact_20260112: team membership lookup failed for \
                                 summary_model=sum access check; denying access: offline";

// one authorization scope's backing object, as a scripted repository shows it
#[derive(Clone, Copy, Debug)]
enum Membership {
    Missing,
    NoBudgetTable,
    EmptyModels,
    Listed,
    Failed,
}

#[derive(Clone, Copy, Debug)]
enum Failure {
    Denied,
    Broken,
}

#[tokio::test]
async fn summary_scope_order() {
    let harness = Harness::full();
    let auth = principal(EVERY_SCOPE);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    assert_eq!(outcome.calls, every_scope_journal());
    assert!(outcome.diagnostics.is_empty());
}

#[rstest]
#[case::empty_lists(
    r#"{"models": [], "team_models": [], "team_model_aliases": {"sum": "team-model"},
        "team_id": "team-1", "user_id": "user-1", "project_id": "project-1"}"#
)]
#[case::absent_lists(
    r#"{"team_model_aliases": {"sum": "team-model"}, "team_id": "team-1",
        "user_id": "user-1", "project_id": "project-1"}"#
)]
#[tokio::test]
async fn empty_key_and_team_allowlists_skip_their_checks(#[case] fields: &str) {
    let harness = Harness::full();
    let auth = principal(fields);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    assert_eq!(outcome.calls, journal_between(2, 8));
}

// a model or alias entry that is not a name is not a grant, so only the string entries reach the check
#[tokio::test]
async fn non_string_model_entries_are_dropped() {
    let harness = Harness::scripted(&[Outcome::Allow; 2]);
    let auth = principal(
        r#"{"models": ["key-model", 7], "team_models": [{"name": "team-model"}, "team-model"],
            "team_model_aliases": {"sum": 3, "other": "team-model"}, "team_id": "team-1"}"#,
    );
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    let aliases = BTreeMap::from([("other".to_owned(), "team-model".to_owned())]);
    let calls = [
        ("key", vec!["key-model".to_owned()]),
        ("team", vec!["team-model".to_owned()]),
    ];
    let expected = calls
        .into_iter()
        .map(|(object_type, models)| Call::Object {
            object_type,
            model: SUM.to_owned(),
            models,
            team_model_aliases: aliases.clone(),
            team_id: Some("team-1".to_owned()),
            router: true,
        })
        .collect::<Vec<_>>();
    assert_eq!(outcome.calls, expected);
    assert!(outcome.diagnostics.is_empty());
}

#[rstest]
#[case::key(0, 1)]
#[case::team(1, 2)]
#[case::user(2, 4)]
#[case::project(3, 6)]
#[case::member(4, 8)]
#[tokio::test]
async fn a_denial_stops_later_scopes(#[case] denying: usize, #[case] kept: usize) {
    let mut outcomes = [Outcome::Allow; 5];
    outcomes[denying] = Outcome::Deny;
    let harness = Harness::scripted(&outcomes);
    let auth = principal(EVERY_SCOPE);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(!outcome.allowed);
    assert_eq!(outcome.calls, journal_between(0, kept));
    assert!(outcome.diagnostics.is_empty());
}

#[tokio::test]
async fn no_principal_or_absent_proxy_allows() {
    let harness = Harness::full();
    let auth = principal(EVERY_SCOPE);
    let without_principal = run(None, Some(&harness.globals(true)), &harness.journal).await;
    assert!(without_principal.allowed);
    assert!(without_principal.calls.is_empty());
    let without_proxy = run(Some(&auth), None, &harness.journal).await;
    assert!(without_proxy.allowed);
    assert!(without_proxy.calls.is_empty());
    assert!(without_proxy.diagnostics.is_empty());
}

// a loaded user is checked even with an empty allowlist, and a project with models=[] skips its check
#[tokio::test]
async fn backing_objects_control_scope() {
    let mut harness = Harness::scripted(&[Outcome::Allow]);
    harness.repositories.user = Lookup::Found(user_row(&[]));
    harness.repositories.project = Lookup::Found(project_row(&[]));
    let auth = principal(r#"{"user_id": "user-1", "project_id": "project-1"}"#);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    assert_eq!(
        outcome.calls,
        vec![
            Call::LookupUser("user-1".to_owned(), false),
            Call::User {
                model: SUM.to_owned(),
                user_models: Vec::new(),
                router: true,
            },
            Call::LookupProject("project-1".to_owned()),
        ]
    );
    assert!(outcome.diagnostics.is_empty());
}

#[tokio::test]
async fn a_missing_user_or_project_skips_its_check() {
    let harness = Harness::empty(&[Outcome::Allow; 2]);
    let auth = principal(r#"{"user_id": "user-1", "project_id": "project-1"}"#);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    assert_eq!(
        outcome.calls,
        vec![
            Call::LookupUser("user-1".to_owned(), false),
            Call::LookupProject("project-1".to_owned()),
        ]
    );
    assert!(outcome.diagnostics.is_empty());
}

#[tokio::test]
async fn null_ids_or_an_absent_database_skip_their_lookups() {
    let harness = Harness::full();
    let auth = principal(r#"{"user_id": null, "team_id": null, "project_id": null}"#);
    let globals = harness.globals(true);
    let null_ids = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(null_ids.allowed);
    assert!(null_ids.calls.is_empty());
    let identified = principal(EVERY_SCOPE);
    let without_database = harness.globals(false);
    let no_database = run(Some(&identified), Some(&without_database), &harness.journal).await;
    assert!(no_database.allowed);
    assert_eq!(no_database.calls, journal_between(0, 2));
}

#[tokio::test]
async fn empty_string_ids_still_look_their_objects_up() {
    let harness = Harness::empty(&[Outcome::Allow; 5]);
    let auth = principal(r#"{"user_id": "", "team_id": "", "project_id": ""}"#);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    assert_eq!(
        outcome.calls,
        vec![
            Call::LookupUser(String::new(), false),
            Call::LookupProject(String::new()),
            Call::LookupMembership(String::new(), String::new()),
        ]
    );
    assert!(outcome.diagnostics.is_empty());
}

#[tokio::test]
async fn a_failed_user_lookup_logs_and_continues() {
    let mut harness = Harness::scripted(&[Outcome::Allow; 2]);
    harness.repositories.user = Lookup::Failed;
    let auth = principal(r#"{"user_id": "user-1", "project_id": "project-1"}"#);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    assert_eq!(
        outcome.calls,
        vec![
            Call::LookupUser("user-1".to_owned(), false),
            Call::LookupProject("project-1".to_owned()),
            Call::Project {
                model: SUM.to_owned(),
                project_models: vec!["team-model".to_owned()],
                router: true,
            },
        ]
    );
    assert_eq!(
        outcome.diagnostics,
        vec![debug(
            "compact_20260112: user object lookup failed for summary_model=sum access check; \
             skipping user-level scope: offline"
        )]
    );
}

#[tokio::test]
async fn a_failed_project_lookup_logs_and_continues() {
    let mut harness = Harness::scripted(&[Outcome::Allow]);
    harness.repositories.project = Lookup::Failed;
    let auth = principal(r#"{"user_id": "user-1", "project_id": "project-1"}"#);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(outcome.allowed);
    assert_eq!(
        outcome.calls,
        vec![
            Call::LookupUser("user-1".to_owned(), false),
            Call::User {
                model: SUM.to_owned(),
                user_models: vec!["key-model".to_owned()],
                router: true,
            },
            Call::LookupProject("project-1".to_owned()),
        ]
    );
    assert_eq!(
        outcome.diagnostics,
        vec![debug(
            "compact_20260112: project object lookup failed for summary_model=sum access \
             check; skipping project-level scope: offline"
        )]
    );
}

#[rstest]
#[case::missing(Membership::Missing, true, false, None)]
#[case::no_budget_table(Membership::NoBudgetTable, true, false, None)]
#[case::empty_models(Membership::EmptyModels, true, false, None)]
#[case::listed(Membership::Listed, true, true, None)]
#[case::failed(Membership::Failed, false, false, Some(MEMBERSHIP_FAILED))]
#[tokio::test]
async fn membership_lookup_is_fail_closed(
    #[case] script: Membership,
    #[case] allowed: bool,
    #[case] checked: bool,
    #[case] diagnostic: Option<&str>,
) {
    let mut harness = Harness::scripted(&[Outcome::Allow; 5]);
    harness.repositories.membership = match script {
        Membership::Missing => Lookup::Missing,
        Membership::Failed => Lookup::Failed,
        Membership::NoBudgetTable => Lookup::Found(membership_row(None)),
        Membership::EmptyModels => Lookup::Found(membership_row(Some(&[]))),
        Membership::Listed => Lookup::Found(membership_row(Some(&["member-model"]))),
    };
    let auth = principal(
        r#"{"user_id": "user-1", "team_id": "team-1", "project_id": "project-1",
            "team_model_aliases": {"sum": "team-model"}}"#,
    );
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert_eq!(outcome.allowed, allowed);
    // the membership lookup runs either way; only a listed membership adds the member check
    let end = if checked { 8 } else { 7 };
    assert_eq!(outcome.calls, journal_between(2, end));
    assert_eq!(
        outcome.diagnostics,
        diagnostic.map_or_else(Vec::new, |text| vec![warn(text)])
    );
}

#[rstest]
#[case::key_denied(0, Failure::Denied, None)]
#[case::team_denied(1, Failure::Denied, None)]
#[case::user_denied(2, Failure::Denied, None)]
#[case::project_denied(3, Failure::Denied, None)]
#[case::member_denied(4, Failure::Denied, None)]
#[case::key_broken(
    0,
    Failure::Broken,
    Some(
        "compact_20260112: unexpected error during key-level access check for \
          summary_model=sum; denying access: broken"
    )
)]
#[case::team_broken(
    1,
    Failure::Broken,
    Some(
        "compact_20260112: unexpected error during team-level access check for \
          summary_model=sum; denying access: broken"
    )
)]
#[case::user_broken(
    2,
    Failure::Broken,
    Some(
        "compact_20260112: unexpected error during user-level access check for \
          summary_model=sum; denying access: broken"
    )
)]
#[case::project_broken(
    3,
    Failure::Broken,
    Some(
        "compact_20260112: unexpected error during project-level access check for \
          summary_model=sum; denying access: broken"
    )
)]
#[case::member_broken(
    4,
    Failure::Broken,
    Some(
        "compact_20260112: unexpected error during member-level access check for \
          summary_model=sum; denying access: broken"
    )
)]
#[tokio::test]
async fn policy_failures_deny(
    #[case] denying: usize,
    #[case] failure: Failure,
    #[case] diagnostic: Option<&str>,
) {
    let mut outcomes = [Outcome::Allow; 5];
    outcomes[denying] = match failure {
        Failure::Denied => Outcome::Deny,
        Failure::Broken => Outcome::Broken,
    };
    let harness = Harness::scripted(&outcomes);
    let auth = principal(EVERY_SCOPE);
    let globals = harness.globals(true);
    let outcome = run(Some(&auth), Some(&globals), &harness.journal).await;
    assert!(!outcome.allowed);
    assert_eq!(
        outcome.diagnostics,
        diagnostic.map_or_else(Vec::new, |text| vec![warn(text)])
    );
}

// a process without the proxy reads null and 4096, and the decoded settings otherwise
#[test]
fn settings_without_a_proxy_use_the_fallbacks() {
    assert_eq!(
        read_summary_settings(None),
        (None, DEFAULT_SUMMARY_MAX_TOKENS)
    );
    let settings = Settings {
        context_management_summary_model: Some(" ".to_owned()),
        context_management_summary_max_tokens: 512,
    };
    assert_eq!(
        read_summary_settings(Some(&settings)),
        (Some(" ".to_owned()), 512)
    );
}
