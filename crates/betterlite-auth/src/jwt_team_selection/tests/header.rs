use std::collections::BTreeMap;
use std::sync::Arc;

use super::support::{Script, headers, ids, lookup, rendered};
use crate::jwt_team_selection::header::{HeaderTeam, resolve_team_from_header, team_header_value};

const TEAM_A: &str = r#"{"team_id": "team-a", "models": ["model-a"]}"#;
const TEAM_B: &str = r#"{"team_id": "team-b", "models": ["model-b"]}"#;
const BROKEN: &str = r#"{"team_id": 5}"#;

#[test]
fn header_missing_empty() {
    assert_eq!(team_header_value(None), None);
    let empty = BTreeMap::new();
    assert_eq!(team_header_value(Some(&empty)), None);
    let blank = headers(&[("x-litellm-team-id", "")]);
    assert_eq!(team_header_value(Some(&blank)), Some(String::new()));
}

#[test]
fn header_casefold() {
    let mixed = headers(&[("X-LiteLLM-Team-Id", "team-a")]);
    assert_eq!(team_header_value(Some(&mixed)), Some("team-a".to_owned()));
}

#[tokio::test]
async fn explicit_team_header() {
    let script = Arc::new(Script::new());
    let lookup = lookup(&script, true);
    let resolved = resolve_team_from_header(
        Some(&headers(&[("X-LiteLLM-TEAM-ID", "team-a")])),
        &ids(&["team-a", "team-b"]),
        false,
        &lookup,
    )
    .await
    .unwrap();
    assert_eq!(
        resolved,
        Some(HeaderTeam {
            header_value: "team-a".to_owned(),
            team_id: "team-a".to_owned(),
        })
    );
    assert!(script.events().is_empty());
}

#[tokio::test]
async fn absent_header_selects_nothing() {
    let script = Arc::new(Script::new());
    let lookup = lookup(&script, true);
    let resolved = resolve_team_from_header(
        Some(&headers(&[("x-litellm-team-id", "")])),
        &ids(&["team-a"]),
        false,
        &lookup,
    )
    .await
    .unwrap();
    assert_eq!(resolved, None);
}

#[tokio::test]
async fn alias_header() {
    let script = Arc::new(Script::new().with_alias("alias-a", &[TEAM_A]));
    let lookup = lookup(&script, true);
    let resolved = resolve_team_from_header(
        Some(&headers(&[("x-litellm-team-id", "alias-a")])),
        &ids(&["team-a"]),
        false,
        &lookup,
    )
    .await
    .unwrap();
    assert_eq!(
        resolved,
        Some(HeaderTeam {
            header_value: "alias-a".to_owned(),
            team_id: "team-a".to_owned(),
        })
    );
    assert_eq!(script.events(), vec!["alias alias-a"]);
}

#[tokio::test]
async fn header_not_in_claims() {
    let script = Arc::new(Script::new().with_alias("alias-b", &[TEAM_B]));
    let lookup = lookup(&script, true);
    let (status, error_type, message) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "alias-b")])),
            &ids(&["team-a", "team-c"]),
            false,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 403);
    assert_eq!(error_type, "auth_error");
    assert_eq!(
        message,
        "x-litellm-team-id 'alias-b' does not resolve to a team id or a unique team alias in your JWT's allowed teams. Allowed team ids: ['team-a', 'team-c']"
    );
}

#[tokio::test]
async fn alias_ambiguous_denied() {
    let script = Arc::new(Script::new().with_alias("alias-a", &[TEAM_A, TEAM_B]));
    let lookup = lookup(&script, true);
    let (status, _, message) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "alias-a")])),
            &ids(&["team-a"]),
            false,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 403);
    assert!(message.contains("Allowed team ids: ['team-a']"));
}

#[tokio::test]
async fn provisional_id_disables_upsert() {
    let script =
        Arc::new(Script::new().with_provisioned(r#"{"team_id": "team-new", "models": []}"#));
    let lookup = lookup(&script, true);
    let (status, _, message) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "team-new")])),
            &ids(&["team-a"]),
            true,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 403);
    assert_eq!(
        message,
        "x-litellm-team-id 'team-new' does not resolve to a team id or a unique team alias among your team memberships."
    );
    assert_eq!(script.events(), vec!["team team-new", "alias team-new"]);
}

#[tokio::test]
async fn absent_id_tries_alias() {
    let script = Arc::new(Script::new().with_alias("alias-a", &[TEAM_A]));
    let lookup = lookup(&script, true);
    let resolved = resolve_team_from_header(
        Some(&headers(&[("x-litellm-team-id", "alias-a")])),
        &ids(&["team-a"]),
        true,
        &lookup,
    )
    .await
    .unwrap();
    assert_eq!(
        resolved,
        Some(HeaderTeam {
            header_value: "alias-a".to_owned(),
            team_id: "team-a".to_owned(),
        })
    );
    assert_eq!(script.events(), vec!["team alias-a", "alias alias-a"]);
}

#[tokio::test]
async fn provisional_present_id_is_accepted() {
    let script = Arc::new(Script::new().with_team(TEAM_B));
    let lookup = lookup(&script, true);
    let resolved = resolve_team_from_header(
        Some(&headers(&[("x-litellm-team-id", "team-b")])),
        &ids(&["team-a"]),
        true,
        &lookup,
    )
    .await
    .unwrap();
    assert_eq!(
        resolved,
        Some(HeaderTeam {
            header_value: "team-b".to_owned(),
            team_id: "team-b".to_owned(),
        })
    );
    assert_eq!(script.events(), vec!["team team-b"]);
}

#[tokio::test]
async fn id_read_error_membership_denial() {
    let script = Arc::new(Script::new().with_raw("team-broken", BROKEN));
    let lookup = lookup(&script, true);
    let (status, _, message) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "team-broken")])),
            &ids(&["team-a"]),
            true,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 403);
    assert_eq!(
        message,
        "x-litellm-team-id 'team-broken' does not resolve to a team id or a unique team alias among your team memberships."
    );
}

// without a database the provisional read is a plain 401, not a membership decision
#[tokio::test]
async fn provisional_no_db_propagates() {
    let script = Arc::new(Script::new());
    let lookup = lookup(&script, false);
    let (status, _, message) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "team-x")])),
            &ids(&["team-a"]),
            true,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 401);
    assert_eq!(
        message,
        "Authentication Error, No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
}

#[tokio::test]
async fn alias_no_db() {
    let script = Arc::new(Script::new().with_alias("alias-a", &[TEAM_A]));
    let lookup = lookup(&script, false);
    let (status, _, _) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "alias-a")])),
            &ids(&["team-a"]),
            false,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 403);
    assert!(script.events().is_empty());
}

#[tokio::test]
async fn alias_client_error_denied() {
    let script = Arc::new(Script::new());
    let lookup = lookup(&script, true);
    let (status, _, _) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "missing")])),
            &ids(&["team-a"]),
            false,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 403);
}

#[tokio::test]
async fn alias_server_error_propagates() {
    let script = Arc::new(Script::new().with_alias("broken", &[BROKEN]));
    let lookup = lookup(&script, true);
    let (status, _, _) = rendered(
        resolve_team_from_header(
            Some(&headers(&[("x-litellm-team-id", "broken")])),
            &ids(&["team-a"]),
            false,
            &lookup,
        )
        .await
        .unwrap_err(),
    );
    assert_eq!(status, 500);
}
