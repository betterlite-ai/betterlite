use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use super::alerts::{BudgetAlert, BudgetAlertKind, BudgetAlertSink, EventGroup, schedule};
use super::email::{dedup_recipients, valid_alert_threshold_config};
use super::money::Money;

pub const TEAM_MEMBER_ALERT_KEY: &str = "team_member_max_budget_alert_emails";

// _team_member_max_budget_alert_check: the team's own map applies with no global merge; a positive spend at or above its lowest threshold alerts
pub fn team_member_alert_check(
    team_id: &str,
    team_alias: Option<&str>,
    team_metadata: Option<&BTreeMap<String, JsonData>>,
    organization_id: Option<&str>,
    user_id: &str,
    user_email: Option<&str>,
    spend: Decimal,
    max_budget: Money,
    alerts: Option<&dyn BudgetAlertSink>,
) {
    let raw_config = team_metadata.and_then(|metadata| metadata.get(TEAM_MEMBER_ALERT_KEY));
    let Some(config) = valid_alert_threshold_config(raw_config) else {
        return;
    };
    if config.is_empty() || spend <= Decimal::ZERO {
        return;
    }
    let merged: BTreeMap<String, Vec<String>> = config
        .into_iter()
        .map(|(threshold, recipients)| (threshold, dedup_recipients(&recipients)))
        .collect();
    let Some(min_pct) = merged
        .keys()
        .filter_map(|threshold| threshold.parse::<i64>().ok())
        .min()
    else {
        return;
    };
    if Money::spend_below(spend, max_budget.mul_percent(min_pct)) {
        return;
    }
    schedule(
        alerts,
        BudgetAlert {
            kind: BudgetAlertKind::MaxBudgetAlert,
            spend,
            max_budget: Some(max_budget),
            soft_budget: None,
            token: None,
            user_id: Some(user_id.to_owned()),
            team_id: Some(team_id.to_owned()),
            team_alias: team_alias.map(str::to_owned),
            organization_id: organization_id.map(str::to_owned),
            user_email: user_email.map(str::to_owned),
            key_alias: None,
            event_group: EventGroup::TeamMember,
            max_budget_alert_emails: Some(merged),
        },
    );
}
