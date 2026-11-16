#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::key_attribution::metadata::KeyMetadata;

use super::metadata::metadata_with_recovered_owner;

#[test]
fn metadata_merge_creates_a_stub_with_only_the_owner_for_a_new_key() {
    let merged = metadata_with_recovered_owner(&BTreeMap::new(), "sk-new", "owner-1");
    assert_eq!(
        merged,
        KeyMetadata {
            key_alias: None,
            team_id: None,
            user_id: Some("owner-1".to_owned()),
            user_email: None,
            key_exists: None,
        }
    );
}

#[test]
fn metadata_merge_replaces_the_owner_and_keeps_every_other_field() {
    let metadata = BTreeMap::from([(
        "sk-known".to_owned(),
        KeyMetadata {
            key_alias: Some("alias".to_owned()),
            team_id: Some("team-a".to_owned()),
            user_id: Some("old-owner".to_owned()),
            user_email: Some("old@example.com".to_owned()),
            key_exists: Some(true),
        },
    )]);
    let merged = metadata_with_recovered_owner(&metadata, "sk-known", "new-owner");
    assert_eq!(
        merged,
        KeyMetadata {
            key_alias: Some("alias".to_owned()),
            team_id: Some("team-a".to_owned()),
            user_id: Some("new-owner".to_owned()),
            user_email: Some("old@example.com".to_owned()),
            key_exists: Some(true),
        }
    );
}

#[test]
fn metadata_merge_leaves_absent_fields_absent_for_a_known_key() {
    let metadata = BTreeMap::from([(
        "sk-partial".to_owned(),
        KeyMetadata {
            key_alias: Some("alias".to_owned()),
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: Some(false),
        },
    )]);
    let merged = metadata_with_recovered_owner(&metadata, "sk-partial", "owner-1");
    assert_eq!(merged.key_alias.as_deref(), Some("alias"));
    assert_eq!(merged.user_id.as_deref(), Some("owner-1"));
    assert!(merged.team_id.is_none());
    assert!(merged.user_email.is_none());
    assert_eq!(merged.key_exists, Some(false));
}

#[test]
fn metadata_merge_reads_only_the_named_key() {
    let metadata = BTreeMap::from([(
        "sk-other".to_owned(),
        KeyMetadata {
            key_alias: Some("other-alias".to_owned()),
            team_id: None,
            user_id: Some("other-owner".to_owned()),
            user_email: None,
            key_exists: None,
        },
    )]);
    let merged = metadata_with_recovered_owner(&metadata, "sk-new", "owner-1");
    assert!(merged.key_alias.is_none());
    assert_eq!(merged.user_id.as_deref(), Some("owner-1"));
    assert_eq!(metadata["sk-other"].user_id.as_deref(), Some("other-owner"));
}
