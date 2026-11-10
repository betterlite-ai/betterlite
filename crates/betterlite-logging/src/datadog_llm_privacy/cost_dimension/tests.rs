#![cfg(test)]

use crate::datadog_llm_privacy::cost_dimension::{cost_dimension_tags, declared_cost_tags};
use crate::datadog_llm_privacy::tests::support::{self, object};

#[test]
fn cost_dimension_tags_normalize_each_value_and_drop_an_empty_one() {
    let mut payload = support::payload();
    payload.metadata.user_api_key_user_id = Some("Alice Smith".to_owned());
    payload.metadata.user_api_key_alias = Some(String::new());
    payload.model_group = Some("GPT-4o Mini".to_owned());
    let router = object(
        r#"{"router_tier": "LOW", "router_cause": "heuristic", "router_escalated": true, "routed_model": "gpt-4o-mini"}"#,
    );
    assert_eq!(
        cost_dimension_tags(&payload, &router),
        vec![
            "user:alice_smith",
            "model_group:gpt-4o_mini",
            "router_tier:low",
            "router_cause:heuristic",
            "router_escalated:true",
            "routed_model:gpt-4o-mini",
        ]
    );
}

#[test]
fn cost_dimension_tags_omit_an_absent_value_and_one_that_normalizes_away() {
    let mut payload = support::payload();
    payload.model_group = None;
    payload.metadata.user_api_key_user_id = Some("  ".to_owned());
    let tags = cost_dimension_tags(&payload, &object(r#"{"router_tier": "low"}"#));
    assert_eq!(tags, vec!["router_tier:low"]);
}

#[test]
fn cost_dimension_tags_read_a_number_and_skip_a_container() {
    let mut payload = support::payload();
    payload.model_group = None;
    let router = object(r#"{"router_tier": 5, "router_cause": ["a"], "routed_model": {}}"#);
    assert_eq!(
        cost_dimension_tags(&payload, &router),
        vec!["router_tier:5"]
    );
}

#[test]
fn declared_cost_tags_list_only_the_dimensions_the_span_carries() {
    let tags = vec![
        "env:prod".to_owned(),
        "team:engineering".to_owned(),
        "user:alice".to_owned(),
        "key_alias:".to_owned(),
        "empty:".to_owned(),
        "no_value_part".to_owned(),
        "router_tier:low".to_owned(),
        "router_signals:a".to_owned(),
    ];
    assert_eq!(
        declared_cost_tags(&tags),
        vec!["team", "user", "router_tier"]
    );
}

#[test]
fn declared_cost_tags_are_empty_without_a_cost_tag() {
    assert!(declared_cost_tags(&[]).is_empty());
    assert!(declared_cost_tags(&["env:prod".to_owned()]).is_empty());
}
