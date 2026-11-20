#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::support::{Logs, membership, stored_membership};
use crate::membership_load::cache::{CachedMembership, CachedPayload, classify_cached};

fn classify(cached: CachedPayload) -> CachedMembership {
    classify_cached(cached).expect("the payload classifies")
}

#[test]
fn cache_none_is_miss() {
    assert_eq!(classify(CachedPayload::from(None)), CachedMembership::Miss);
}

#[test]
fn exact_sentinel_is_absence() {
    let sentinel = Json::Text("__no_team_membership__".to_owned());
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(sentinel))),
        CachedMembership::Absent
    );
}

#[test]
fn typed_membership_present() {
    let typed = membership();
    assert_eq!(
        classify(CachedPayload::Typed(Box::new(typed.clone()))),
        CachedMembership::Membership(Box::new(typed))
    );
}

#[test]
fn dictionary_membership_present() {
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(stored_membership()))),
        CachedMembership::Membership(Box::new(membership()))
    );
}

#[test]
fn malformed_dictionary_is_miss() {
    let empty = serde_json::from_str::<Json>("{}").expect("the empty mapping");
    let wrong_user = serde_json::from_str::<Json>(r#"{"user_id":7,"team_id":"team-1"}"#)
        .expect("the invalid membership");
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(empty))),
        CachedMembership::Miss
    );
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(wrong_user))),
        CachedMembership::Miss
    );
}

#[test]
fn scalar_is_miss_without_warning() {
    let (logs, _guard) = Logs::capture();
    let scalars = [
        Json::Null,
        Json::Bool(false),
        Json::Number(Number::from(0_u8)),
        Json::Text("__no_team_membership".to_owned()),
        Json::List(Vec::new()),
    ];
    let misses: Vec<CachedMembership> = scalars
        .into_iter()
        .map(|payload| classify(CachedPayload::Stored(Box::new(payload))))
        .collect();
    assert_eq!(misses, vec![CachedMembership::Miss; 5]);
    assert_eq!(logs.text(), "");
}

#[test]
fn codec_warning_preserved() {
    let (logs, _guard) = Logs::capture();
    let invalid = serde_json::from_str::<Json>(r#"{"user_id":7,"team_id":"team-1"}"#)
        .expect("the invalid membership");
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(invalid))),
        CachedMembership::Miss
    );
    let text = logs.text();
    assert!(
        text.contains("CacheCodec.decode: validation failed"),
        "{text}"
    );
    assert!(text.contains("LiteLLMTeamMembership"), "{text}");
}

#[test]
fn negative_result_differs_from_miss() {
    let sentinel = Json::Text("__no_team_membership__".to_owned());
    let absent = classify(CachedPayload::Stored(Box::new(sentinel)));
    let miss = classify(CachedPayload::Pending);
    assert_eq!(absent, CachedMembership::Absent);
    assert_eq!(miss, CachedMembership::Miss);
    assert_ne!(absent, miss);
}
