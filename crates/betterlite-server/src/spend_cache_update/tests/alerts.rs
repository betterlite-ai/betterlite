use std::sync::Arc;

use rust_decimal::Decimal;

use crate::failure_hook::ports::{BudgetAlertKind, CallInfo, EntityType};

use super::super::alerts::SpendBudgetAlerts as _;
use super::support::{RecordingSlack, proxy_alerts};

#[tokio::test]
async fn the_proxy_alerts_port_sends_the_projected_alert_through_slack() {
    let slack = Arc::new(RecordingSlack::default());
    let alerts = proxy_alerts(Arc::clone(&slack));
    let info = CallInfo {
        spend: Decimal::from(1400),
        max_budget: Some(Decimal::from(2000)),
        soft_budget: None,
        token: Some("cached-token".to_owned()),
        customer_id: None,
        user_id: Some("user-1".to_owned()),
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: None,
        key_alias: Some("alias".to_owned()),
        projected_exceeded_date: Some("2024-06-21".to_owned()),
        projected_spend: Some(Decimal::from(2900)),
        event_group: EntityType::Key,
        alert_emails: None,
        max_budget_alert_emails: None,
    };
    alerts.projected_limit_exceeded(info.clone()).await;
    assert_eq!(
        slack.budget_calls(),
        [(BudgetAlertKind::ProjectedLimitExceeded, info)]
    );
}
