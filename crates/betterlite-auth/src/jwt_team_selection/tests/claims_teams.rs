use std::sync::Arc;

use super::support::{Script, claims, ids, lookup, rendered, selection, settings};
use crate::jwt_team_selection::claims_teams::{
    find_and_validate_specific_team_id, get_all_team_ids,
};

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
    let (team_id, row) = find_and_validate_specific_team_id(&jwt, &lookup, &required_team(), None)
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
    let (team_id, _row) = find_and_validate_specific_team_id(&jwt, &lookup, &required_team(), None)
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
    let (team_id, _row) = find_and_validate_specific_team_id(&jwt, &lookup, &settings, Some(true))
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
        find_and_validate_specific_team_id(&jwt, &lookup, &settings, None)
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
        find_and_validate_specific_team_id(&jwt, &lookup, &fallback, None)
            .await
            .unwrap(),
        (None, None)
    );
    let strict = required_team();
    let (status, _, _) = rendered(
        find_and_validate_specific_team_id(&jwt, &lookup, &strict, None)
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
        find_and_validate_specific_team_id(&jwt, &lookup, &required_team(), None)
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
async fn required_team_missing() {
    let script = Arc::new(Script::new());
    let fields = claim_fields();
    let document = serde_json::json!({"sub": "someone"});
    let jwt = claims(document, &fields);
    let lookup = lookup(&script, true);
    let (status, error_type, message) = rendered(
        find_and_validate_specific_team_id(&jwt, &lookup, &required_team(), None)
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
        find_and_validate_specific_team_id(&jwt, &lookup, &hint_settings("roles.0"), None)
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
        find_and_validate_specific_team_id(&jwt, &lookup, &hint_settings("roles[0]"), None)
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 401);
    assert_eq!(
        message,
        "Authentication Error, No team found in token. Checked team_id field 'roles[0]' and team_alias field 'None'. Hint: array indexing (e.g. 'roles[0]') is not supported in team_id_jwt_field. Use 'roles' instead — LiteLLM automatically uses the first element when the field value is a list."
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
