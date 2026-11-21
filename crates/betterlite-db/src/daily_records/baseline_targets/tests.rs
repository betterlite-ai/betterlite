#![cfg(test)]

use std::collections::BTreeMap;

use crate::daily_records::support::{Case, base_payload, daily_payload, payload_with};

use super::{BaselineTargetEntity, DailyBaselineAttribution, baseline_daily_attribution};

fn case() -> Case {
    Case::with_prices(BTreeMap::new())
}

async fn attribution(case: &Case, text: &str) -> Option<DailyBaselineAttribution> {
    baseline_daily_attribution(&daily_payload(text), &case.pricing(), &case.endpoints)
        .await
        .unwrap()
}

fn listed_targets(
    attribution: &DailyBaselineAttribution,
) -> Vec<(BaselineTargetEntity, Option<String>)> {
    attribution
        .targets
        .iter()
        .map(|target| (target.entity, target.entity_id.clone()))
        .collect()
}

#[tokio::test]
async fn lists_every_identity_of_every_available_base_in_table_order() {
    let text = payload_with(
        r#""team_id":"t1","organization_id":"o1","end_user":"e1","agent_id":"a1","request_tags":["b","a"]"#,
    );
    let attribution = attribution(&case(), &text)
        .await
        .expect("the payload names a base");
    assert_eq!(
        listed_targets(&attribution),
        vec![
            (BaselineTargetEntity::User, Some("u1".to_owned())),
            (BaselineTargetEntity::Team, Some("t1".to_owned())),
            (BaselineTargetEntity::Org, Some("o1".to_owned())),
            (BaselineTargetEntity::EndUser, Some("e1".to_owned())),
            (BaselineTargetEntity::Agent, Some("a1".to_owned())),
            (BaselineTargetEntity::Tag, Some("b".to_owned())),
            (BaselineTargetEntity::Tag, Some("a".to_owned())),
        ]
    );
    assert_eq!(attribution.date, "2025-01-02");
    assert_eq!(attribution.api_key, "k1");
}

#[tokio::test]
async fn falls_past_the_missing_user_to_the_team_base() {
    let text = r#"{"team_id":"t1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","spend":0.25}"#;
    let attribution = attribution(&case(), text)
        .await
        .expect("the team names a base");
    assert_eq!(
        listed_targets(&attribution),
        vec![(BaselineTargetEntity::Team, Some("t1".to_owned()))]
    );
    assert_eq!(attribution.model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn reaches_the_tag_base_after_every_other_identity_is_absent() {
    let text = r#"{"request_tags":["x"],"startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","spend":0.25}"#;
    let attribution = attribution(&case(), text)
        .await
        .expect("the tag names a base");
    assert_eq!(
        listed_targets(&attribution),
        vec![(BaselineTargetEntity::Tag, Some("x".to_owned()))]
    );
    assert_eq!(attribution.date, "2025-01-02");
}

#[tokio::test]
async fn builds_the_base_from_the_end_user_the_source_normalizes() {
    let text = r#"{"end_user":"e1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","spend":0.25}"#;
    let attribution = attribution(&case(), text)
        .await
        .expect("the normalized end user names a base");
    assert_eq!(
        listed_targets(&attribution),
        vec![(BaselineTargetEntity::EndUser, Some("e1".to_owned()))]
    );
}

#[tokio::test]
async fn lists_a_null_user_as_a_target_without_an_id() {
    let mut payload = daily_payload(&base_payload());
    payload.user = Some(None);
    let case = case();
    let attribution = baseline_daily_attribution(&payload, &case.pricing(), &case.endpoints)
        .await
        .unwrap()
        .expect("a present null user still names a base");
    assert_eq!(
        listed_targets(&attribution),
        vec![(BaselineTargetEntity::User, None)]
    );
}

#[tokio::test]
async fn lists_no_target_for_an_empty_end_user() {
    let text = payload_with(r#""end_user":"""#);
    let attribution = attribution(&case(), &text)
        .await
        .expect("the user names a base");
    assert_eq!(
        listed_targets(&attribution),
        vec![(BaselineTargetEntity::User, Some("u1".to_owned()))]
    );
}

#[tokio::test]
async fn carries_the_endpoint_and_tool_name_of_the_base() {
    let mut case = case();
    case.endpoints.routes.insert(
        "aimage_generation".to_owned(),
        "/image/generations".to_owned(),
    );
    let text = payload_with(r#""call_type":"aimage_generation","mcp_namespaced_tool_name":"tool""#);
    let attribution = attribution(&case, &text)
        .await
        .expect("the user names a base");
    assert_eq!(attribution.endpoint.as_deref(), Some("/image/generations"));
    assert_eq!(
        attribution.mcp_namespaced_tool_name.as_deref(),
        Some("tool")
    );
    assert_eq!(attribution.model_group.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn returns_nothing_when_no_entity_names_a_base() {
    let text = r#"{"startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","spend":0.25}"#;
    assert!(attribution(&case(), text).await.is_none());
}
