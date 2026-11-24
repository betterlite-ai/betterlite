use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use super::alerts::{BudgetAlert, BudgetAlertKind, BudgetAlertSink, EventGroup, schedule};
use super::email::{dedup_recipients, valid_alert_threshold_config};
use super::money::Money;

pub const TEAM_MEMBER_ALERT_KEY: &str = "team_member_max_budget_alert_emails";

// the team, user and spend one team-member alert is built from
pub struct TeamMemberAlert<'request> {
    pub team_id: &'request str,
    pub team_alias: Option<&'request str>,
    pub team_metadata: Option<&'request BTreeMap<String, JsonData>>,
    pub organization_id: Option<&'request str>,
    pub user_id: &'request str,
    pub user_email: Option<&'request str>,
    pub spend: Decimal,
    pub max_budget: Money,
}

// _team_member_max_budget_alert_check: the team's own map applies with no global merge; a positive spend at or above its lowest threshold alerts
pub fn team_member_alert_check(alert: &TeamMemberAlert<'_>, alerts: Option<&dyn BudgetAlertSink>) {
    let raw_config = alert
        .team_metadata
        .and_then(|metadata| metadata.get(TEAM_MEMBER_ALERT_KEY));
    let Some(config) = valid_alert_threshold_config(raw_config) else {
        return;
    };
    if config.is_empty() || alert.spend <= Decimal::ZERO {
        return;
    }
    let merged = dedup_config(config);
    let Some(min_pct) = minimum_threshold(&merged) else {
        return;
    };
    if Money::spend_below(alert.spend, alert.max_budget.mul_percent(min_pct)) {
        return;
    }
    schedule(alerts, team_alert(alert, merged));
}

fn dedup_config(config: BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    config
        .into_iter()
        .map(|(threshold, recipients)| (threshold, dedup_recipients(&recipients)))
        .collect()
}

fn minimum_threshold(config: &BTreeMap<String, Vec<String>>) -> Option<i64> {
    config
        .keys()
        .filter_map(|threshold| threshold.parse::<i64>().ok())
        .min()
}

fn team_alert(alert: &TeamMemberAlert<'_>, config: BTreeMap<String, Vec<String>>) -> BudgetAlert {
    BudgetAlert {
        kind: BudgetAlertKind::MaxBudgetAlert,
        spend: alert.spend,
        max_budget: Some(alert.max_budget),
        soft_budget: None,
        token: None,
        user_id: Some(alert.user_id.to_owned()),
        team_id: Some(alert.team_id.to_owned()),
        team_alias: alert.team_alias.map(str::to_owned),
        organization_id: alert.organization_id.map(str::to_owned),
        user_email: alert.user_email.map(str::to_owned),
        key_alias: None,
        event_group: EventGroup::TeamMember,
        max_budget_alert_emails: Some(config),
    }
}
