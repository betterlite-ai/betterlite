use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use tracing::debug;

use crate::auth_key_budgets::email::parse_email_list;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::alerts::{TeamAlertKind, TeamAlertSink, token_alert};

pub const SOFT_BUDGET_EMAILS_FIELD: &str = "soft_budget_alerting_emails";

// what the soft check did; it never denies, so a caller only reads it to see which log or alert happened
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoftOutcome {
    NotCrossed,
    CrossedWithoutToken,
    SkippedWithoutRecipients,
    Alerted,
}

// _team_soft_budget_check: the row's spend at or above soft_budget logs, and only the team's own recipients authorize an alert
pub fn team_soft_budget_check(
    team: Option<&LiteLLMTeamTable>,
    token: Option<&UserAPIKeyAuth>,
    alerts: Option<&dyn TeamAlertSink>,
) -> SoftOutcome {
    let Some(team) = team else {
        return SoftOutcome::NotCrossed;
    };
    let base = &team.fields.base;
    let (Some(soft_budget), Some(spend)) = (base.soft_budget, team.fields.spend) else {
        return SoftOutcome::NotCrossed;
    };
    if spend < soft_budget {
        return SoftOutcome::NotCrossed;
    }
    debug!(team_id = %base.team_id, spend = %spend, soft_budget = %soft_budget, "Crossed Soft Budget for team");
    let Some(token) = token else {
        return SoftOutcome::CrossedWithoutToken;
    };
    let Some(recipients) = soft_budget_recipients(base.metadata.as_ref()) else {
        debug!(
            team_id = %base.team_id,
            "Skipping team soft budget alert: no alert_emails configured in metadata.soft_budget_alerting_emails"
        );
        return SoftOutcome::SkippedWithoutRecipients;
    };
    if let Some(sink) = alerts {
        sink.schedule(token_alert(
            TeamAlertKind::SoftBudget,
            token,
            team,
            spend,
            Some(recipients),
        ));
    }
    SoftOutcome::Alerted
}

// a string splits on commas and trims; a list keeps its nonblank strings untrimmed; any other shape authorizes nobody
#[must_use]
pub fn soft_budget_recipients(
    metadata: Option<&BTreeMap<String, JsonData>>,
) -> Option<Vec<String>> {
    let configured = metadata?.get(SOFT_BUDGET_EMAILS_FIELD)?;
    let recipients = match configured {
        JsonData::String(_) => parse_email_list(configured),
        JsonData::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                JsonData::String(email) if !email.trim().is_empty() => Some(email.clone()),
                JsonData::Null
                | JsonData::Bool(_)
                | JsonData::Number(_)
                | JsonData::String(_)
                | JsonData::Array(_)
                | JsonData::Object(_) => None,
            })
            .collect(),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Object(_) => {
            return None;
        }
    };
    Some(recipients).filter(|emails| !emails.is_empty())
}
