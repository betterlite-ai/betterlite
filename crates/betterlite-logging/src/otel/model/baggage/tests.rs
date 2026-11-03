#![cfg(test)]
use crate::otel::model::baggage::{
    BAGGAGE_PROMOTED_KEYS, TEAM_METADATA, filtered_team_metadata_json, promoted_baggage,
    promoted_metadata,
};
use crate::otel::model::metadata::identity::RequestIdentity;
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

fn identity() -> RequestIdentity {
    let team_metadata: BTreeMap<String, OtelModelValue> =
        serde_json::from_str(r#"{"region":"west","secret":"x"}"#).expect("test JSON is valid");
    RequestIdentity {
        call_id: None,
        team_id: Some("team".to_owned()),
        team_alias: None,
        team_metadata: Some(team_metadata),
        key_hash: None,
        end_user: None,
        provider_model: None,
        request_route: None,
        metadata: [
            ("requester_metadata.user_id".to_owned(), "user".to_owned()),
            ("private".to_owned(), "x".to_owned()),
        ]
        .into_iter()
        .collect(),
    }
}

#[test]
fn baggage_filters_identity_metadata_and_team_metadata() {
    let identity = identity();
    let promoted = promoted_baggage(
        &identity,
        Some("group"),
        &BAGGAGE_PROMOTED_KEYS
            .iter()
            .map(|key| (*key).to_owned())
            .collect::<Vec<_>>(),
        &["requester_metadata.user_id".to_owned()],
        &["region".to_owned()],
    );
    assert_eq!(
        promoted.get("litellm.team.id").map(String::as_str),
        Some("team")
    );
    assert_eq!(
        promoted.get("litellm.team.metadata").map(String::as_str),
        Some(r#"{"region": "west"}"#)
    );
    assert_eq!(
        promoted.get("litellm.metadata.user_id").map(String::as_str),
        Some("user")
    );
    assert!(!promoted.contains_key("litellm.metadata.private"));
}

#[test]
fn empty_team_metadata_is_dropped() {
    assert_eq!(
        filtered_team_metadata_json(None, &["region".to_owned()]),
        None
    );
    assert_eq!(
        promoted_metadata(&BTreeMap::new(), &["x".to_owned()]).len(),
        0
    );
    assert_eq!(TEAM_METADATA, "litellm.team.metadata");
}
