#![cfg(test)]
use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::{agent_caller_auth, caller_record};
use crate::agent_policy::caller::AgentCaller;
use crate::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};

fn agent_key(forwarded: Option<AgentCaller>, requires_fresh_policy: bool) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(
        r#"{"agent_id": "agent-1", "user_id": "agent-owner", "team_id": "agent-team",
            "models": ["gpt-4o"], "key_alias": "agent key", "user_role": "proxy_admin",
            "object_permission_id": "perm-1"}"#,
    )
    .unwrap();
    let mut key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    key.extra.agent_caller = forwarded;
    key.extra.requires_fresh_policy = requires_fresh_policy;
    key.extra.parent_otel_span = Some(ParentOtelSpan::Json(JsonData::String("span-1".to_owned())));
    key
}

fn caller(user_id: Option<&str>, team_id: Option<&str>) -> AgentCaller {
    AgentCaller {
        user_id: user_id.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
    }
}

#[rstest]
#[case::plain(false)]
#[case::fresh_policy(true)]
fn a_key_without_a_caller_has_no_stand_in(#[case] requires_fresh_policy: bool) {
    let key = agent_key(None, requires_fresh_policy);
    assert!(agent_caller_auth(&key).unwrap().is_none());
}

#[test]
fn the_stand_in_holds_only_the_callers_ids() {
    let key = agent_key(Some(caller(Some("human"), Some("humans"))), false);
    let record = agent_caller_auth(&key).unwrap().unwrap();
    assert_eq!(record.view.token.user_id.as_deref(), Some("human"));
    assert_eq!(record.view.token.team_id.as_deref(), Some("humans"));
    assert_eq!(record.view.token.agent_id, None);
    assert!(record.view.token.models.is_empty());
    assert_eq!(record.view.token.key_alias, None);
    assert_eq!(record.view.token.object_permission_id, None);
    assert!(record.extra.user_role.is_none());
    assert!(record.extra.agent_caller.is_none());
}

#[rstest]
#[case::user_only(Some("human"), None)]
#[case::team_only(None, Some("humans"))]
fn an_absent_caller_id_stays_absent(#[case] user_id: Option<&str>, #[case] team_id: Option<&str>) {
    let key = agent_key(Some(caller(user_id, team_id)), false);
    let record = agent_caller_auth(&key).unwrap().unwrap();
    assert_eq!(record.view.token.user_id.as_deref(), user_id);
    assert_eq!(record.view.token.team_id.as_deref(), team_id);
}

#[rstest]
#[case::plain(false)]
#[case::fresh_policy(true)]
fn the_stand_in_copies_the_keys_requires_fresh_policy(#[case] requires_fresh_policy: bool) {
    let key = agent_key(Some(caller(Some("human"), None)), requires_fresh_policy);
    let record = agent_caller_auth(&key).unwrap().unwrap();
    assert_eq!(record.extra.requires_fresh_policy, requires_fresh_policy);
}

#[test]
fn the_stand_in_carries_the_keys_trace_handle() {
    let key = agent_key(Some(caller(Some("human"), None)), false);
    let record = agent_caller_auth(&key).unwrap().unwrap();
    let carried = record.extra.parent_otel_span;
    assert!(matches!(
        carried.as_ref(),
        Some(ParentOtelSpan::Json(JsonData::String(span))) if span == "span-1"
    ));
}

#[test]
fn a_record_built_without_a_span_has_none() {
    let record = caller_record(&caller(Some("human"), None), true, None).unwrap();
    assert!(record.extra.parent_otel_span.is_none());
    assert!(record.extra.requires_fresh_policy);
}
