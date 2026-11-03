#![cfg(test)]
use crate::otel::model::baggage::{
    BAGGAGE_PROMOTED_KEYS, TEAM_METADATA, filtered_team_metadata_json, promoted_baggage,
    promoted_metadata,
};
use crate::otel::model::metadata::request_identity::RequestIdentity;
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
        serde_json::from_str::<BTreeMap<String, String>>(
            promoted
                .get(TEAM_METADATA)
                .expect("allowlisted team metadata")
        )
        .unwrap(),
        BTreeMap::from([("region".to_owned(), "west".to_owned())])
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

#[test]
fn end_user_is_opt_in_and_team_metadata_needs_its_own_allowlist() {
    let mut caller = identity();
    caller.end_user = Some("person".to_owned());
    let defaults: Vec<String> = BAGGAGE_PROMOTED_KEYS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    let promoted = promoted_baggage(&caller, None, &defaults, &[], &[]);
    assert!(!promoted.contains_key("litellm.end_user.id"));
    assert!(!promoted.contains_key(TEAM_METADATA));
    let opted_in = promoted_baggage(&caller, None, &["litellm.end_user.id".to_owned()], &[], &[]);
    assert_eq!(opted_in.len(), 1);
    assert_eq!(
        opted_in.get("litellm.end_user.id").map(String::as_str),
        Some("person")
    );
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
struct NestedMetadata {
    text: String,
    items: (Option<bool>, bool, u64),
}

#[test]
fn team_metadata_round_trips_nested_values() {
    let source = r#"{"public":{"text":"quote\"\n\\","items":[null,true,9007199254740993]},"private":"hidden"}"#;
    let metadata: BTreeMap<String, OtelModelValue> = serde_json::from_str(source).unwrap();
    let encoded = filtered_team_metadata_json(Some(&metadata), &["public".to_owned()]).unwrap();
    let decoded: BTreeMap<String, NestedMetadata> = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(
        decoded.into_values().next(),
        Some(NestedMetadata {
            text: "quote\"\n\\".to_owned(),
            items: (None, true, 0x0020_0000_0000_0001),
        })
    );
}
