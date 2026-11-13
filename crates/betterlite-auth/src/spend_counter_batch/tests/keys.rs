use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;

use crate::spend_counter_batch::keys::{
    CounterId, EntityCounterIds, PostCallCounterKeys, admission_counter_keys, entity_counter_keys,
    post_call_counter_keys,
};

use super::support::user_auth;

fn blank_ids() -> EntityCounterIds<'static> {
    EntityCounterIds {
        token: CounterId::Unusable,
        team_id: CounterId::Unusable,
        user_id: CounterId::Unusable,
        org_id: CounterId::Unusable,
        project_id: CounterId::Unusable,
        end_user_id: CounterId::Unusable,
    }
}

#[test]
fn all_entity_counter_formats() {
    let mut ids = blank_ids();
    ids.token = CounterId::Text("token-1");
    ids.team_id = CounterId::Text("team-1");
    ids.user_id = CounterId::Text("user-1");
    ids.org_id = CounterId::Text("org-1");
    ids.project_id = CounterId::Text("project-1");
    ids.end_user_id = CounterId::Text("end-user-1");
    assert_eq!(
        entity_counter_keys(&ids),
        BTreeSet::from([
            "spend:key:token-1".to_owned(),
            "spend:team:team-1".to_owned(),
            "spend:team_member:user-1:team-1".to_owned(),
            "spend:user:user-1".to_owned(),
            "spend:end_user:end-user-1".to_owned(),
            "spend:org:org-1".to_owned(),
            "spend:project:project-1".to_owned(),
        ])
    );
}

#[test]
fn only_string_entity_ids() {
    assert!(entity_counter_keys(&blank_ids()).is_empty());
    let json = JsonData::Number(serde_json::Number::from(7_i64));
    let ids = EntityCounterIds {
        token: CounterId::from_json(Some(&json)),
        team_id: CounterId::from_json(Some(&json)),
        user_id: CounterId::from_json(Some(&json)),
        org_id: CounterId::from_json(Some(&json)),
        project_id: CounterId::from_json(Some(&json)),
        end_user_id: CounterId::from_json(Some(&json)),
    };
    assert!(entity_counter_keys(&ids).is_empty());
}

#[test]
fn empty_entity_ids_kept() {
    let mut ids = blank_ids();
    ids.token = CounterId::Text("");
    ids.team_id = CounterId::Text("");
    ids.user_id = CounterId::Text("");
    ids.org_id = CounterId::Text("");
    ids.project_id = CounterId::Text("");
    ids.end_user_id = CounterId::Text("");
    assert_eq!(
        entity_counter_keys(&ids),
        BTreeSet::from([
            "spend:key:".to_owned(),
            "spend:team:".to_owned(),
            "spend:team_member::".to_owned(),
            "spend:user:".to_owned(),
            "spend:end_user:".to_owned(),
            "spend:org:".to_owned(),
            "spend:project:".to_owned(),
        ])
    );
}

#[test]
fn team_member_requires_two_string_ids() {
    let mut team_only = blank_ids();
    team_only.team_id = CounterId::Text("team-1");
    assert_eq!(
        entity_counter_keys(&team_only),
        BTreeSet::from(["spend:team:team-1".to_owned()])
    );
    let mut user_only = blank_ids();
    user_only.user_id = CounterId::Text("user-1");
    assert_eq!(
        entity_counter_keys(&user_only),
        BTreeSet::from(["spend:user:user-1".to_owned()])
    );
}

#[test]
fn delimiters_not_escaped() {
    let mut ids = blank_ids();
    ids.token = CounterId::Text("token:1");
    ids.team_id = CounterId::Text("team{1}");
    ids.user_id = CounterId::Text("user:1");
    let keys = entity_counter_keys(&ids);
    assert!(keys.contains("spend:key:token:1"));
    assert!(keys.contains("spend:team:team{1}"));
    assert!(keys.contains("spend:team_member:user:1:team{1}"));
    assert!(keys.contains("spend:user:user:1"));
}

#[test]
fn token_not_rehashed() {
    let mut ids = blank_ids();
    ids.token = CounterId::Text("sk-abc123");
    assert_eq!(
        entity_counter_keys(&ids),
        BTreeSet::from(["spend:key:sk-abc123".to_owned()])
    );
}

#[test]
fn admission_uses_explicit_end_user() {
    let auth = user_auth(&[
        ("token", "token-1"),
        ("team_id", "team-1"),
        ("user_id", "user-1"),
        ("end_user_id", "view-end-user"),
    ]);
    let keys = admission_counter_keys(&auth, Some("request-end-user"));
    assert!(keys.contains("spend:end_user:request-end-user"));
    assert!(!keys.contains("spend:end_user:view-end-user"));
    assert!(keys.contains("spend:key:token-1"));
    assert!(keys.contains("spend:team:team-1"));
    assert!(keys.contains("spend:team_member:user-1:team-1"));
    assert!(keys.contains("spend:user:user-1"));
}

#[test]
fn post_call_tags_and_groups_truthy_strings() {
    let tags = [
        JsonData::String("alpha".to_owned()),
        JsonData::String(String::new()),
        JsonData::Bool(true),
        JsonData::Number(serde_json::Number::from(3_i64)),
        JsonData::Null,
        JsonData::String("beta".to_owned()),
    ];
    let groups = [
        JsonData::String("group-1".to_owned()),
        JsonData::String(String::new()),
        JsonData::Bool(false),
    ];
    let keys = post_call_counter_keys(&PostCallCounterKeys {
        entity: blank_ids(),
        tags: &tags,
        model_access_groups: &groups,
    });
    assert_eq!(
        keys,
        BTreeSet::from([
            "spend:model_access_group:group-1".to_owned(),
            "spend:tag:alpha".to_owned(),
            "spend:tag:beta".to_owned(),
        ])
    );
}

#[test]
fn post_call_project_default_none() {
    let mut ids = blank_ids();
    ids.token = CounterId::Text("token-1");
    ids.project_id = CounterId::from_json(None);
    let keys = post_call_counter_keys(&PostCallCounterKeys {
        entity: ids,
        tags: &[],
        model_access_groups: &[],
    });
    assert_eq!(keys, BTreeSet::from(["spend:key:token-1".to_owned()]));
}

#[test]
fn counter_sets_deduplicate() {
    let tags = [
        JsonData::String("dup".to_owned()),
        JsonData::String("dup".to_owned()),
    ];
    let groups = [
        JsonData::String("dup".to_owned()),
        JsonData::String("dup".to_owned()),
    ];
    let mut ids = blank_ids();
    ids.token = CounterId::Text("token-1");
    let keys = post_call_counter_keys(&PostCallCounterKeys {
        entity: ids,
        tags: &tags,
        model_access_groups: &groups,
    });
    assert_eq!(
        keys,
        BTreeSet::from([
            "spend:key:token-1".to_owned(),
            "spend:model_access_group:dup".to_owned(),
            "spend:tag:dup".to_owned(),
        ])
    );
}
