#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::records::OrgRecord;
use rust_decimal::Decimal;

use super::super::tests::support::{
    DEPLOYMENT_FAILURES, FAILED, ORG_BUDGET, ZERO_COST, dec, error_information, event, fixture,
    fixture_with_settings, labelled_settings, payload, rich_payload, series_label, totals,
};

#[tokio::test]
async fn invalid_key_failure_is_skipped() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.error_information = Some(error_information("invalid api key"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert!(totals(&fixture.registry, FAILED).is_empty());
    assert!(totals(&fixture.registry, DEPLOYMENT_FAILURES).is_empty());
    assert!(fixture.lookups.orgs().is_empty());
}

#[tokio::test]
async fn failed_counter_deployment_and_org_budget_run() {
    let fixture = fixture();
    fixture.lookups.set_org_record(OrgRecord {
        spend: Some(dec("10")),
        organization_alias: Some("org-alias".to_owned()),
        litellm_budget_table: Some(BudgetTableSnapshot {
            max_budget: Some(dec("100")),
            budget_reset_at: None,
        }),
    });
    let mut record = payload(dec("15"));
    record.metadata.user_api_key_org_id = Some("org-1".to_owned());
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(totals(&fixture.registry, FAILED), vec![Decimal::ONE]);
    assert_eq!(
        totals(&fixture.registry, DEPLOYMENT_FAILURES),
        vec![Decimal::ONE]
    );
    assert_eq!(fixture.lookups.orgs(), vec!["org-1".to_owned()]);
    // the failure event asks for the org gauge with a zero cost, so the stored spend alone comes off the budget
    assert_eq!(totals(&fixture.registry, ORG_BUDGET), vec![dec("90")]);
}

#[tokio::test]
async fn failure_labels_match_litellm() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(Decimal::ZERO);
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, FAILED, "model"), "gpt-4o");
    assert_eq!(series_label(registry, FAILED, "model_id"), "deployment-1");
    assert_eq!(series_label(registry, FAILED, "end_user"), "end-user-1");
    assert_eq!(series_label(registry, FAILED, "user"), "user-1");
    assert_eq!(series_label(registry, FAILED, "hashed_api_key"), "sk-live");
    assert_eq!(series_label(registry, FAILED, "api_key_alias"), "alias-1");
    assert_eq!(series_label(registry, FAILED, "team"), "team-1");
    assert_eq!(series_label(registry, FAILED, "team_alias"), "team-alias");
    // the failure counter carries only LiteLLM's LLM_API_FAILED schema
    let failure_labels = registry
        .get(FAILED)
        .expect("the failed metric is registered")
        .description
        .label_names
        .clone();
    assert!(!failure_labels.contains(&"status_code".to_owned()));
    assert!(!failure_labels.contains(&"litellm_model_name".to_owned()));
    assert!(!failure_labels.contains(&"route".to_owned()));
    assert!(!failure_labels.contains(&"user_email".to_owned()));
    assert!(!failure_labels.contains(&"requested_model".to_owned()));
    assert!(!failure_labels.contains(&"client_ip".to_owned()));
    assert!(!failure_labels.contains(&"user_agent".to_owned()));
}

#[tokio::test]
async fn deployment_failure_names_the_deployment() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(Decimal::ZERO);
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "model_group"),
        "gpt-4-group"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "requested_model"),
        "gpt-4-group"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "litellm_model_name"),
        "gpt-4o"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "model_id"),
        "deployment-1"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "api_base"),
        "https://api.example/v1"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "api_provider"),
        "openai"
    );
}

#[tokio::test]
async fn deployment_failure_carries_the_exception_and_request_labels() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(Decimal::ZERO);
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "exception_status"),
        "None"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "exception_class"),
        "BadRequestError"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "client_ip"),
        "10.0.0.7"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "user_agent"),
        "test-agent"
    );
}

#[tokio::test]
async fn deployment_failure_carries_the_principal_labels() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(Decimal::ZERO);
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "hashed_api_key"),
        "sk-live"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "api_key_alias"),
        "alias-1"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "team"),
        "team-1"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "team_alias"),
        "team-alias"
    );
}

#[tokio::test]
async fn a_failure_without_a_model_id_labels_the_empty_text() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.model_group = Some("gpt-4-group".to_owned());
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, FAILED, "model_id"), "");
    assert_eq!(series_label(registry, FAILED, "model"), "gpt-4o");
    // without a model id the deployment step labels no deployment and writes the same empty text
    assert_eq!(series_label(registry, DEPLOYMENT_FAILURES, "model_id"), "");
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "litellm_model_name"),
        ""
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "requested_model"),
        "gpt-4o"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "model_group"),
        "gpt-4-group"
    );
}

#[tokio::test]
async fn a_bedrock_failure_is_labelled_without_the_provider_prefix() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.model = "bedrock/claude-3".to_owned();
    record.custom_llm_provider = Some("bedrock".to_owned());
    record.model_id = Some("deployment-1".to_owned());
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, FAILED, "model"), "claude-3");
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "litellm_model_name"),
        "claude-3"
    );
}

#[tokio::test]
async fn a_cohere_chat_failure_is_labelled_without_the_declared_prefix() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.model = "cohere/command-r".to_owned();
    record.custom_llm_provider = Some("cohere_chat".to_owned());
    record.model_id = Some("deployment-1".to_owned());
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, FAILED, "model"), "command-r");
    assert_eq!(
        series_label(registry, DEPLOYMENT_FAILURES, "litellm_model_name"),
        "command-r"
    );
}

#[tokio::test]
async fn the_failure_event_writes_the_zero_cost_counter() {
    let fixture = fixture();
    let mut record = rich_payload(Decimal::ZERO);
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    // the failure event runs the zero-cost emitter with the payload's diagnostic
    assert_eq!(totals(&fixture.registry, ZERO_COST), vec![Decimal::ONE]);
}

#[tokio::test]
async fn errors_inside_the_failure_event_are_swallowed() {
    let fixture = fixture();
    fixture.lookups.fail_org_info();
    let mut record = payload(Decimal::ZERO);
    record.metadata.user_api_key_org_id = Some("org-1".to_owned());
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    // the failed counter ran before the org lookup, and the store error never escaped the event
    assert_eq!(fixture.lookups.orgs(), vec!["org-1".to_owned()]);
    assert_eq!(totals(&fixture.registry, FAILED), vec![Decimal::ONE]);
    assert!(totals(&fixture.registry, ORG_BUDGET).is_empty());
}
