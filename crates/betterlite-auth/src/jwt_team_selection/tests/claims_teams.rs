use std::sync::Arc;

use super::support::{Script, claims, ids, lookup, rendered, selection, settings};
use crate::jwt_team_selection::claims_teams::{find_team_in_claims, get_all_team_ids};

const TEAM_A: &str = r#"{"team_id": "team-a", "models": ["model-a"]}"#;
const DEFAULT_TEAM: &str = r#"{"team_id": "default-team", "models": []}"#;
const CREATED_TEAM: &str = r#"{"team_id": "team-new", "models": []}"#;

fn claim_fields() -> crate::jwt_claims::settings::JwtAuthSettings {
    settings(|fields| {
        fields.team_id_jwt_field = Some("team_id".to_owned());
        fields.team_alias_jwt_field = Some("team_alias".to_owned());
        fields.team_ids_jwt_field = Some("team_ids".to_owned());
    })
}

fn required_team() -> super::super::settings::SelectionSettings {
    selection(|fields| {
        fields.teams.team_id_jwt_field = Some("team_id".to_owned());
        fields.teams.team_alias_jwt_field = Some("team_alias".to_owned());
        fields.teams.team_ids_jwt_field = Some("team_ids".to_owned());
    })
}

#[tokio::test]
async fn team_id_before_alias() {
    let script = Arc::new(
        Script::new()
            .with_team(TEAM_A)
            .with_alias("alias-a", &[TEAM_A]),
    );
    let fields = claim_fields();
    let document = serde_json::json!({"team_id": "team-a", "team_alias": "alias-a"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let (team_id, row) = find_team_in_claims(&jwt, &lookup, &required_team(), None)
        .await
        .unwrap();
    assert_eq!(team_id.as_deref(), Some("team-a"));
    assert_eq!(row.unwrap().fields.base.team_id, "team-a");
    assert_eq!(script.events(), vec!["team team-a"]);
}

#[tokio::test]
async fn alias_before_default() {
    let script = Arc::new(
        Script::new()
            .with_team(TEAM_A)
            .with_team(DEFAULT_TEAM)
            .with_alias("alias-a", &[TEAM_A]),
    );
    let fields = settings(|fields| {
        fields.team_id_jwt_field = Some("team_id".to_owned());
        fields.team_alias_jwt_field = Some("team_alias".to_owned());
        fields.team_id_default = Some("default-team".to_owned());
    });
    let document = serde_json::json!({"team_alias": "alias-a"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let (team_id, _row) = find_team_in_claims(&jwt, &lookup, &required_team(), None)
        .await
        .unwrap();
    assert_eq!(team_id.as_deref(), Some("team-a"));
    assert_eq!(script.events(), vec!["alias alias-a"]);
}

#[tokio::test]
async fn explicit_upsert_override() {
    let script = Arc::new(Script::new().with_provisioned(CREATED_TEAM));
    let fields = claim_fields();
    let document = serde_json::json!({"team_id": "team-new"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let settings = required_team();
    let (team_id, _row) = find_team_in_claims(&jwt, &lookup, &settings, Some(true))
        .await
        .unwrap();
    assert_eq!(team_id.as_deref(), Some("team-new"));
    assert_eq!(script.events(), vec!["team team-new", "provision team-new"]);
}

#[tokio::test]
async fn explicit_upsert_absent_without_override() {
    let script = Arc::new(Script::new().with_provisioned(CREATED_TEAM));
    let fields = claim_fields();
    let document = serde_json::json!({"team_id": "team-new"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let settings = required_team();
    let (status, _, _) = rendered(
        find_team_in_claims(&jwt, &lookup, &settings, None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 404);
    assert_eq!(script.events(), vec!["team team-new"]);
}

#[tokio::test]
async fn claim_404_fallback() {
    let script = Arc::new(Script::new());
    let fields = claim_fields();
    let document = serde_json::json!({"team_id": "missing"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let fallback = selection(|fields| fields.routing.team_claim_fallback = true);
    assert_eq!(
        find_team_in_claims(&jwt, &lookup, &fallback, None)
            .await
            .unwrap(),
        (None, None)
    );
    let strict = required_team();
    let (status, _, _) = rendered(
        find_team_in_claims(&jwt, &lookup, &strict, None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 404);
}

#[tokio::test]
async fn claim_non404_propagates() {
    let script = Arc::new(Script::new());
    let fields = claim_fields();
    let document = serde_json::json!({"team_id": "team-a"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, false);
    let (status, _, message) = rendered(
        find_team_in_claims(&jwt, &lookup, &required_team(), None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 401);
    assert_eq!(
        message,
        "Authentication Error, No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
    // the fallback only swallows a proven 404, so the same read failure still propagates
    let fallback = selection(|fields| fields.routing.team_claim_fallback = true);
    let (status, _, _) = rendered(
        find_team_in_claims(&jwt, &lookup, &fallback, None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 401);
}

#[tokio::test]
async fn required_team_missing() {
    let script = Arc::new(Script::new());
    let fields = claim_fields();
    let document = serde_json::json!({"sub": "someone"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let (status, error_type, message) = rendered(
        find_team_in_claims(&jwt, &lookup, &required_team(), None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 401);
    assert_eq!(error_type, "auth_error");
    assert_eq!(
        message,
        "Authentication Error, No team found in token. Checked team_id field 'team_id' and team_alias field 'team_alias'."
    );
}

fn hint_settings(field: &str) -> super::super::settings::SelectionSettings {
    selection(|fields| fields.teams.team_id_jwt_field = Some(field.to_owned()))
}

#[tokio::test]
async fn dot_index_hint() {
    let script = Arc::new(Script::new());
    let fields = settings(|fields| fields.team_id_jwt_field = Some("roles.0".to_owned()));
    let document = serde_json::json!({"sub": "someone"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let (status, _, message) = rendered(
        find_team_in_claims(&jwt, &lookup, &hint_settings("roles.0"), None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 401);
    assert_eq!(
        message,
        "Authentication Error, No team found in token. Checked team_id field 'roles.0' and team_alias field 'None'. Hint: dot-notation array indexing (e.g. 'roles.0') is not supported. Use 'roles' instead — LiteLLM automatically uses the first element when the field value is a list."
    );
}

#[tokio::test]
async fn bracket_index_hint() {
    let script = Arc::new(Script::new());
    let fields = settings(|fields| fields.team_id_jwt_field = Some("roles[0]".to_owned()));
    let document = serde_json::json!({"sub": "someone"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let (status, _, message) = rendered(
        find_team_in_claims(&jwt, &lookup, &hint_settings("roles[0]"), None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 401);
    assert_eq!(
        message,
        "Authentication Error, No team found in token. Checked team_id field 'roles[0]' and team_alias field 'None'. Hint: array indexing (e.g. 'roles[0]') is not supported in team_id_jwt_field. Use 'roles' instead — LiteLLM automatically uses the first element when the field value is a list."
    );
}

async fn hint_message(field: &str) -> String {
    let script = Arc::new(Script::new());
    let lookup = lookup(&script, true);
    let fields = settings(|fields| fields.team_id_jwt_field = Some(field.to_owned()));
    let jwt = claims(serde_json::json!({"sub": "someone"}), &fields);
    rendered(
        find_team_in_claims(&jwt, &lookup, &hint_settings(field), None)
            .await
            .unwrap_err(),
    )
    .2
}

// only a real list index gets a hint: a non-numeric suffix, an empty base and an empty index stay plain
#[tokio::test]
async fn index_hint_needs_a_real_index() {
    assert_eq!(
        hint_message("roles.x").await,
        "Authentication Error, No team found in token. Checked team_id field 'roles.x' and team_alias field 'None'."
    );
    assert_eq!(
        hint_message("[0]").await,
        "Authentication Error, No team found in token. Checked team_id field '[0]' and team_alias field 'None'."
    );
    assert!(
        hint_message("team_roles[0]")
            .await
            .contains("Hint: array indexing (e.g. 'team_roles[0]')")
    );
    // a bracket needs a real index too: a non-numeric index and an empty one stay plain
    assert_eq!(
        hint_message("team_roles[x]").await,
        "Authentication Error, No team found in token. Checked team_id field 'team_roles[x]' and team_alias field 'None'."
    );
    assert_eq!(
        hint_message("team_roles[]").await,
        "Authentication Error, No team found in token. Checked team_id field 'team_roles[]' and team_alias field 'None'."
    );
}

// the deviation: an index must be ASCII, where Python's isdigit also accepts digits from other scripts
#[tokio::test]
async fn index_hint_ignores_non_ascii_digits() {
    assert_eq!(
        hint_message("roles.٣").await,
        "Authentication Error, No team found in token. Checked team_id field 'roles.٣' and team_alias field 'None'."
    );
    assert_eq!(
        hint_message("roles[٣]").await,
        "Authentication Error, No team found in token. Checked team_id field 'roles[٣]' and team_alias field 'None'."
    );
}

#[tokio::test]
async fn group_set_deduplicates() {
    let fields = claim_fields();
    let document = serde_json::json!({
        "team_ids": ["team-a", "team-a", "team-b"],
        "team_id": "team-c",
    });
    let jwt = claims(document, &fields);
    assert_eq!(get_all_team_ids(&jwt).unwrap(), ids(&["team-a", "team-b"]));
}
