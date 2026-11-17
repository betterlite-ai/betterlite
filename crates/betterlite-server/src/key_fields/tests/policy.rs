use betterlite_db::budget_permission_rows::JsonData;

use super::{assert_field, value};
use crate::key_fields::bulk::BulkUpdateTeamKeysRequest;
use crate::key_fields::error::DecodeError;
use crate::key_fields::policy::{CustomKeyPolicyRequest, PolicyRequestBody};

#[test]
fn selection_exclusive() {
    assert!(matches!(
        BulkUpdateTeamKeysRequest::read(&value(
            r#"{"team_id": "t", "key_ids": ["k"], "all_keys_in_team": true, "update_fields": {"tags": []}}"#
        )),
        Err(DecodeError::SelectionConflict)
    ));
    assert!(matches!(
        BulkUpdateTeamKeysRequest::read(&value(
            r#"{"team_id": "t", "update_fields": {"tags": []}}"#
        )),
        Err(DecodeError::SelectionMissing)
    ));
    let request = BulkUpdateTeamKeysRequest::read(&value(
        r#"{"team_id": "t", "key_ids": [], "all_keys_in_team": true, "update_fields": {"tags": []}}"#,
    ))
    .expect("all keys with an empty key list");
    let resolved = request.resolved_object().expect("the request dump");
    assert_eq!(resolved.len(), 4);
    assert_field(&resolved, "all_keys_in_team", "true");
    assert_field(&resolved, "key_ids", "[]");
    let Some(JsonData::Object(updates)) = resolved.get("update_fields") else {
        panic!("update_fields is an object");
    };
    assert_eq!(updates.len(), 17);
    assert_field(updates, "tags", "[]");
    let missing_team = BulkUpdateTeamKeysRequest::read(&value(
        r#"{"key_ids": ["k"], "update_fields": {"tags": []}}"#,
    ));
    missing_team.unwrap_err();
}

#[test]
fn custom_policy_record() {
    let generate = CustomKeyPolicyRequest::read(&value(
        r#"{"operation": "generate", "existing_key": null, "effective_key": {}, "request": {"team_id": "t"}}"#,
    ))
    .expect("a generate policy record");
    let projection = generate.project().expect("the policy projection");
    assert_field(&projection, "operation", r#""generate""#);
    assert_field(&projection, "request_kind", r#""GenerateKeyRequest""#);
    assert!(matches!(generate.request, PolicyRequestBody::Generate(_)));
    let update = CustomKeyPolicyRequest::read(&value(concat!(
        r#"{"operation": "update", "existing_key": {"token": "k"}, "effective_key": {},"#,
        r#" "request": {"key": "k", "temp_budget_increase": null, "temp_budget_expiry": null}}"#
    )))
    .expect("an update policy record");
    assert!(matches!(update.request, PolicyRequestBody::Update(_)));
    let rotated = CustomKeyPolicyRequest::read(&value(
        r#"{"operation": "regenerate", "existing_key": null, "effective_key": {}, "request": {"new_key": "n"}}"#,
    ))
    .expect("a regenerate policy record");
    assert!(matches!(rotated.request, PolicyRequestBody::Regenerate(_)));
    let tied = CustomKeyPolicyRequest::read(&value(
        r#"{"operation": "generate", "existing_key": null, "effective_key": {}, "request": {"key": "k"}}"#,
    ))
    .expect("a tied union");
    assert!(matches!(tied.request, PolicyRequestBody::Generate(_)));
}

#[test]
fn custom_policy_rejects() {
    assert!(matches!(
        CustomKeyPolicyRequest::read(&value(
            r#"{"operation": "generate", "effective_key": {}, "request": {}}"#
        )),
        Err(DecodeError::MissingField {
            field: "existing_key"
        })
    ));
    assert!(
        CustomKeyPolicyRequest::read(&value(
            r#"{"operation": "rotate", "existing_key": null, "effective_key": {}, "request": {}}"#
        ))
        .is_err()
    );
    assert!(matches!(
        CustomKeyPolicyRequest::read(&value(concat!(
            r#"{"operation": "generate", "existing_key": null, "effective_key": {},"#,
            r#" "request": {"default_estimated_output_tokens": 0}}"#
        ))),
        Err(DecodeError::RequestUnion)
    ));
}
