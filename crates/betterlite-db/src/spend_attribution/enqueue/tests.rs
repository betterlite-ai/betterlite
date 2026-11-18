#![cfg(test)]

use std::fs::read;
use std::path::PathBuf;
use std::str::FromStr as _;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::spend_attribution::support::{FixtureDeployments, RecordingQueue};
use crate::spend_attribution::tags::{
    EnqueueTagsError, MAX_ENQUEUE_TAGS_BYTES, RequestTags, decode_enqueue_tags,
};
use crate::spend_queues::entity::{EntityType, SpendAmount, SpendUpdateQueueItem};

use super::{
    org_member_transaction_key, update_agent_db, update_model_access_group_db, update_org_db,
    update_project_db, update_tag_db, update_team_db,
};

fn amount(text: &str) -> SpendAmount {
    SpendAmount(Decimal::from_str(text).unwrap())
}

fn queued(entity_type: EntityType, entity_id: &str, cost: &str) -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: Some(entity_type),
        entity_id: Some(entity_id.to_owned()),
        response_cost: Some(amount(cost)),
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[tokio::test]
async fn team_queues_the_team_then_the_member() {
    let queue = RecordingQueue::new(None);
    update_team_db(
        Some(&queue),
        Some(amount("1.25")),
        Some("team-1"),
        Some("user-1"),
    )
    .await
    .unwrap();
    assert_eq!(
        queue.items(),
        vec![
            queued(EntityType::Team, "team-1", "1.25"),
            queued(
                EntityType::TeamMember,
                "team_id::team-1::user_id::user-1",
                "1.25"
            ),
        ]
    );
}

#[tokio::test]
async fn team_member_failure_is_logged_and_swallowed() {
    let queue = RecordingQueue::new(Some(1));
    update_team_db(
        Some(&queue),
        Some(amount("1")),
        Some("team-1"),
        Some("user-1"),
    )
    .await
    .unwrap();
    assert_eq!(queue.items(), vec![queued(EntityType::Team, "team-1", "1")]);
}

#[tokio::test]
async fn team_failure_is_logged_and_propagated() {
    let queue = RecordingQueue::new(Some(0));
    let error = update_team_db(
        Some(&queue),
        Some(amount("1")),
        Some("team-1"),
        Some("user-1"),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        super::super::SpendAttributionError::Enqueue { entity: "team", .. }
    ));
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn team_missing_id_or_database_queues_nothing() {
    let queue = RecordingQueue::new(None);
    update_team_db(Some(&queue), Some(amount("1")), None, Some("user-1"))
        .await
        .unwrap();
    update_team_db(None, Some(amount("1")), Some("team-1"), Some("user-1"))
        .await
        .unwrap();
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn team_empty_id_is_still_queued() {
    let queue = RecordingQueue::new(None);
    update_team_db(Some(&queue), None, Some(""), Some("user-1"))
        .await
        .unwrap();
    assert_eq!(
        queue.items(),
        vec![
            SpendUpdateQueueItem {
                entity_type: Some(EntityType::Team),
                entity_id: Some(String::new()),
                response_cost: None,
            },
            SpendUpdateQueueItem {
                entity_type: Some(EntityType::TeamMember),
                entity_id: Some("team_id::::user_id::user-1".to_owned()),
                response_cost: None,
            },
        ]
    );
}

#[tokio::test]
async fn team_without_a_user_queues_only_the_team() {
    let queue = RecordingQueue::new(None);
    update_team_db(Some(&queue), Some(amount("1")), Some("team-1"), None)
        .await
        .unwrap();
    assert_eq!(queue.items(), vec![queued(EntityType::Team, "team-1", "1")]);
}

#[test]
fn org_member_keys_percent_encode_both_ids() {
    assert_eq!(
        org_member_transaction_key("o::ne/ü", "u ser"),
        "organization_id::o%3A%3Ane%2F%C3%BC::user_id::u%20ser"
    );
    assert_eq!(
        org_member_transaction_key("a_.-~b", "plain"),
        "organization_id::a_.-~b::user_id::plain"
    );
}

#[tokio::test]
async fn org_queues_the_org_then_the_encoded_member() {
    let queue = RecordingQueue::new(None);
    update_org_db(
        Some(&queue),
        Some(amount("2")),
        Some("org::1"),
        Some("user 1"),
    )
    .await
    .unwrap();
    assert_eq!(
        queue.items(),
        vec![
            queued(EntityType::Organization, "org::1", "2"),
            queued(
                EntityType::OrganizationMember,
                "organization_id::org%3A%3A1::user_id::user%201",
                "2"
            ),
        ]
    );
}

#[tokio::test]
async fn org_member_failure_is_logged_and_propagated() {
    let queue = RecordingQueue::new(Some(1));
    let error = update_org_db(
        Some(&queue),
        Some(amount("2")),
        Some("org-1"),
        Some("user-1"),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        super::super::SpendAttributionError::Enqueue {
            entity: "organization member",
            ..
        }
    ));
    assert_eq!(
        queue.items(),
        vec![queued(EntityType::Organization, "org-1", "2")]
    );
}

#[tokio::test]
async fn org_failure_is_logged_and_propagated() {
    let queue = RecordingQueue::new(Some(0));
    let error = update_org_db(
        Some(&queue),
        Some(amount("2")),
        Some("org-1"),
        Some("user-1"),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        super::super::SpendAttributionError::Enqueue {
            entity: "organization",
            ..
        }
    ));
}

#[tokio::test]
async fn org_missing_id_or_database_queues_nothing() {
    let queue = RecordingQueue::new(None);
    update_org_db(Some(&queue), Some(amount("2")), None, Some("user-1"))
        .await
        .unwrap();
    update_org_db(None, Some(amount("2")), Some("org-1"), Some("user-1"))
        .await
        .unwrap();
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn project_queues_the_project_and_propagates_failure() {
    let queue = RecordingQueue::new(None);
    update_project_db(Some(&queue), Some(amount("3")), Some("project-1"))
        .await
        .unwrap();
    assert_eq!(
        queue.items(),
        vec![queued(EntityType::Project, "project-1", "3")]
    );
    let failing = RecordingQueue::new(Some(0));
    let error = update_project_db(Some(&failing), Some(amount("3")), Some("project-1"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        super::super::SpendAttributionError::Enqueue {
            entity: "project",
            ..
        }
    ));
}

#[tokio::test]
async fn project_missing_id_or_database_queues_nothing() {
    let queue = RecordingQueue::new(None);
    update_project_db(Some(&queue), Some(amount("3")), None)
        .await
        .unwrap();
    update_project_db(None, Some(amount("3")), Some("project-1"))
        .await
        .unwrap();
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn agent_queues_the_agent_and_propagates_failure() {
    let queue = RecordingQueue::new(None);
    update_agent_db(Some(&queue), Some(amount("4")), Some("agent-1"))
        .await
        .unwrap();
    assert_eq!(
        queue.items(),
        vec![queued(EntityType::Agent, "agent-1", "4")]
    );
    let failing = RecordingQueue::new(Some(0));
    let error = update_agent_db(Some(&failing), Some(amount("4")), Some("agent-1"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        super::super::SpendAttributionError::Enqueue {
            entity: "agent",
            ..
        }
    ));
}

#[tokio::test]
async fn agent_missing_id_or_database_queues_nothing() {
    let queue = RecordingQueue::new(None);
    update_agent_db(Some(&queue), Some(amount("4")), None)
        .await
        .unwrap();
    update_agent_db(None, Some(amount("4")), Some("agent-1"))
        .await
        .unwrap();
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn serialized_tags_keep_duplicates_and_skip_empty_names() {
    let queue = RecordingQueue::new(None);
    update_tag_db(
        Some(&queue),
        Some(amount("0.5")),
        Some(RequestTags::Serialized(r#"["prod","","prod","test"]"#)),
    )
    .await
    .unwrap();
    assert_eq!(
        queue.items(),
        vec![
            queued(EntityType::Tag, "prod", "0.5"),
            queued(EntityType::Tag, "prod", "0.5"),
            queued(EntityType::Tag, "test", "0.5"),
        ]
    );
}

#[tokio::test]
async fn an_object_payload_queues_its_keys() {
    let queue = RecordingQueue::new(None);
    update_tag_db(
        Some(&queue),
        None,
        Some(RequestTags::Serialized(r#"{"beta":1,"alpha":2}"#)),
    )
    .await
    .unwrap();
    assert_eq!(
        queue.items(),
        vec![
            SpendUpdateQueueItem {
                entity_type: Some(EntityType::Tag),
                entity_id: Some("beta".to_owned()),
                response_cost: None,
            },
            SpendUpdateQueueItem {
                entity_type: Some(EntityType::Tag),
                entity_id: Some("alpha".to_owned()),
                response_cost: None,
            },
        ]
    );
}

#[tokio::test]
async fn a_string_payload_queues_its_characters() {
    let queue = RecordingQueue::new(None);
    update_tag_db(Some(&queue), None, Some(RequestTags::Serialized(r#""ab""#)))
        .await
        .unwrap();
    assert_eq!(
        queue.items(),
        vec![
            SpendUpdateQueueItem {
                entity_type: Some(EntityType::Tag),
                entity_id: Some("a".to_owned()),
                response_cost: None,
            },
            SpendUpdateQueueItem {
                entity_type: Some(EntityType::Tag),
                entity_id: Some("b".to_owned()),
                response_cost: None,
            },
        ]
    );
}

#[tokio::test]
async fn malformed_tag_json_queues_nothing() {
    let queue = RecordingQueue::new(None);
    update_tag_db(
        Some(&queue),
        Some(amount("1")),
        Some(RequestTags::Serialized("[oops")),
    )
    .await
    .unwrap();
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn a_truthy_scalar_tag_payload_fails() {
    let queue = RecordingQueue::new(None);
    let error = update_tag_db(
        Some(&queue),
        Some(amount("1")),
        Some(RequestTags::Serialized("5")),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        super::super::SpendAttributionError::Tags {
            source: EnqueueTagsError::NotIterable,
        }
    ));
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn parsed_tags_are_queued_as_sent_minus_empty_names() {
    let queue = RecordingQueue::new(None);
    let tags = names(&["x", "", "x"]);
    update_tag_db(
        Some(&queue),
        Some(amount("1")),
        Some(RequestTags::Parsed(&tags)),
    )
    .await
    .unwrap();
    assert_eq!(
        queue.items(),
        vec![
            queued(EntityType::Tag, "x", "1"),
            queued(EntityType::Tag, "x", "1"),
        ]
    );
}

#[tokio::test]
async fn tag_failure_is_logged_and_propagated() {
    let queue = RecordingQueue::new(Some(1));
    let error = update_tag_db(
        Some(&queue),
        Some(amount("1")),
        Some(RequestTags::Serialized(r#"["a","b"]"#)),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        super::super::SpendAttributionError::Enqueue { entity: "tag", .. }
    ));
    assert_eq!(queue.items(), vec![queued(EntityType::Tag, "a", "1")]);
}

#[tokio::test]
async fn missing_tags_or_database_queues_nothing() {
    let queue = RecordingQueue::new(None);
    update_tag_db(Some(&queue), Some(amount("1")), None)
        .await
        .unwrap();
    update_tag_db(
        None,
        Some(amount("1")),
        Some(RequestTags::Serialized(r#"["a"]"#)),
    )
    .await
    .unwrap();
    assert!(queue.items().is_empty());
}

#[tokio::test]
async fn access_group_debits_each_surviving_group_the_full_cost() {
    let queue = RecordingQueue::new(None);
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["b","c"]}"#)]);
    let attributed = names(&["a", "b", "c"]);
    update_model_access_group_db(
        Some(&queue),
        Some(amount("2.5")),
        &attributed,
        Some("dep-1"),
        Some(&deployments),
    )
    .await;
    assert_eq!(
        queue.items(),
        vec![
            queued(EntityType::ModelAccessGroup, "b", "2.5"),
            queued(EntityType::ModelAccessGroup, "c", "2.5"),
        ]
    );
}

#[tokio::test]
async fn access_group_enqueue_failure_is_swallowed() {
    let queue = RecordingQueue::new(Some(1));
    let attributed = names(&["a", "b"]);
    update_model_access_group_db(Some(&queue), Some(amount("2.5")), &attributed, None, None).await;
    assert_eq!(
        queue.items(),
        vec![queued(EntityType::ModelAccessGroup, "a", "2.5")]
    );
}

#[tokio::test]
async fn access_group_missing_database_queues_nothing() {
    let queue = RecordingQueue::new(None);
    let attributed = names(&["a"]);
    update_model_access_group_db(None, Some(amount("2.5")), &attributed, None, None).await;
    assert!(queue.items().is_empty());
}

#[test]
fn tag_decoding_keeps_array_members_in_order_with_duplicates() {
    assert_eq!(
        decode_enqueue_tags(br#"["prod","prod","test"]"#).unwrap(),
        names(&["prod", "prod", "test"])
    );
}

#[test]
fn tag_decoding_drops_non_string_list_members() {
    assert_eq!(
        decode_enqueue_tags(br#"["a",1,null,true,["b"],{"c":1}]"#).unwrap(),
        names(&["a"])
    );
}

#[test]
fn tag_decoding_iterates_object_keys_in_order() {
    assert_eq!(
        decode_enqueue_tags(br#"{"beta":1,"alpha":2,"":3}"#).unwrap(),
        names(&["beta", "alpha", ""])
    );
}

#[test]
fn tag_decoding_iterates_string_characters() {
    assert_eq!(
        decode_enqueue_tags("\"aü\"".as_bytes()).unwrap(),
        names(&["a", "ü"])
    );
}

#[test]
fn tag_decoding_treats_malformed_and_falsey_payloads_as_empty() {
    for payload in [
        b"[oops".as_slice(),
        b"[]".as_slice(),
        b"{}".as_slice(),
        b"\"\"".as_slice(),
        b"null".as_slice(),
        b"false".as_slice(),
        b"0".as_slice(),
        b"0.0".as_slice(),
        b"-0.0".as_slice(),
    ] {
        assert_eq!(decode_enqueue_tags(payload).unwrap(), Vec::<String>::new());
    }
}

#[test]
fn tag_decoding_rejects_truthy_scalars() {
    for payload in [b"true".as_slice(), b"5".as_slice(), b"-3.5".as_slice()] {
        assert!(matches!(
            decode_enqueue_tags(payload).unwrap_err(),
            EnqueueTagsError::NotIterable
        ));
    }
}

#[test]
fn tag_decoding_treats_invalid_utf8_as_malformed() {
    assert_eq!(
        decode_enqueue_tags(&[0xff, 0xfe, 0x00]).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn tag_decoding_bounds_the_payload_size() {
    let at_limit = vec![b'['; MAX_ENQUEUE_TAGS_BYTES];
    assert_eq!(
        decode_enqueue_tags(&at_limit).unwrap(),
        Vec::<String>::new()
    );
    let over_limit = vec![b'['; MAX_ENQUEUE_TAGS_BYTES + 1];
    assert!(matches!(
        decode_enqueue_tags(&over_limit).unwrap_err(),
        EnqueueTagsError::TooLarge { len } if len == MAX_ENQUEUE_TAGS_BYTES + 1
    ));
}

#[test]
fn tag_decoding_bounds_the_nesting_depth() {
    let at_limit = format!("{}{}", "[".repeat(64), "]".repeat(64));
    assert_eq!(
        decode_enqueue_tags(at_limit.as_bytes()).unwrap(),
        Vec::<String>::new()
    );
    let over_limit = format!("{}{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_enqueue_tags(over_limit.as_bytes()).unwrap_err(),
        EnqueueTagsError::TooDeep
    ));
}

#[rstest]
fn replays_the_enqueue_tags_corpus(#[files("fuzz/corpus/enqueue_tags/*")] path: PathBuf) {
    drop(decode_enqueue_tags(&read(path).unwrap()));
}
