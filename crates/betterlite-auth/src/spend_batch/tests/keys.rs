use betterlite_db::budget_permission_rows::JsonData;

use crate::spend_batch::keys::{
    CounterEntities, PostCallCounters, admission_counter_keys, entity_counter_keys,
    post_call_counter_keys,
};

use super::support::{keys, user_auth};

const ALL_IDS: CounterEntities<'static> = CounterEntities {
    key_token: Some("sk-1"),
    team: Some("team-1"),
    user: Some("user-1"),
    end_user: Some("end-1"),
    org: Some("org-1"),
    project: Some("proj-1"),
};

#[test]
fn entity_keys_cover_every_counter_family() {
    assert_eq!(
        entity_counter_keys(&ALL_IDS),
        keys(&[
            "spend:key:sk-1",
            "spend:team:team-1",
            "spend:team_member:user-1:team-1",
            "spend:user:user-1",
            "spend:end_user:end-1",
            "spend:org:org-1",
            "spend:project:proj-1",
        ])
    );
}

#[test]
fn empty_ids_still_name_their_counters() {
    let entities = CounterEntities {
        key_token: Some(""),
        team: Some(""),
        user: Some(""),
        end_user: Some(""),
        org: Some(""),
        project: Some(""),
    };
    assert_eq!(
        entity_counter_keys(&entities),
        keys(&[
            "spend:key:",
            "spend:team:",
            "spend:team_member::",
            "spend:user:",
            "spend:end_user:",
            "spend:org:",
            "spend:project:",
        ])
    );
}

#[test]
fn team_member_counter_needs_both_team_and_user() {
    let team_only = CounterEntities {
        team: Some("team-1"),
        ..CounterEntities::default()
    };
    assert_eq!(
        entity_counter_keys(&team_only),
        keys(&["spend:team:team-1"])
    );
    let user_only = CounterEntities {
        user: Some("user-1"),
        ..CounterEntities::default()
    };
    assert_eq!(
        entity_counter_keys(&user_only),
        keys(&["spend:user:user-1"])
    );
    let both = CounterEntities {
        team: Some("team-1"),
        user: Some("user-1"),
        ..CounterEntities::default()
    };
    assert_eq!(
        entity_counter_keys(&both),
        keys(&[
            "spend:team:team-1",
            "spend:team_member:user-1:team-1",
            "spend:user:user-1"
        ])
    );
}

#[test]
fn post_call_filters_and_deduplicates_tags_and_groups() {
    let tags = [
        JsonData::String("beta".to_owned()),
        JsonData::String(String::new()),
        JsonData::String("alpha".to_owned()),
        JsonData::String("beta".to_owned()),
        JsonData::Number(serde_json::Number::from(7)),
        JsonData::Bool(true),
        JsonData::Null,
    ];
    let access_groups = [
        JsonData::String("group-b".to_owned()),
        JsonData::String("group-a".to_owned()),
        JsonData::String(String::new()),
        JsonData::String("group-b".to_owned()),
        JsonData::Number(serde_json::Number::from(3)),
    ];
    let counters = PostCallCounters {
        entities: CounterEntities {
            project: Some("proj-1"),
            ..CounterEntities::default()
        },
        tags: &tags,
        access_groups: &access_groups,
    };
    assert_eq!(
        post_call_counter_keys(&counters),
        keys(&[
            "spend:project:proj-1",
            "spend:tag:alpha",
            "spend:tag:beta",
            "spend:model_access_group:group-a",
            "spend:model_access_group:group-b",
        ])
    );
}

#[test]
fn admission_reads_the_token_view_and_the_end_user_argument() {
    let auth = user_auth(&[
        ("token", "tok-1"),
        ("team_id", "team-1"),
        ("user_id", "user-1"),
        ("org_id", "org-1"),
        ("project_id", "proj-1"),
        ("end_user_id", "from-token"),
    ]);
    assert_eq!(
        admission_counter_keys(&auth, Some("end-2")),
        keys(&[
            "spend:key:tok-1",
            "spend:team:team-1",
            "spend:team_member:user-1:team-1",
            "spend:user:user-1",
            "spend:end_user:end-2",
            "spend:org:org-1",
            "spend:project:proj-1",
        ])
    );
}

#[test]
fn admission_without_ids_names_no_counters() {
    let auth = user_auth(&[]);
    assert!(admission_counter_keys(&auth, None).is_empty());
}

#[test]
fn admission_accepts_empty_id_strings() {
    let auth = user_auth(&[
        ("token", ""),
        ("team_id", ""),
        ("user_id", ""),
        ("org_id", ""),
        ("project_id", ""),
    ]);
    assert_eq!(
        admission_counter_keys(&auth, Some("")),
        keys(&[
            "spend:key:",
            "spend:team:",
            "spend:team_member::",
            "spend:user:",
            "spend:end_user:",
            "spend:org:",
            "spend:project:",
        ])
    );
}
