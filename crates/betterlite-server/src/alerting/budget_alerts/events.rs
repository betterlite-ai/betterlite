use std::fmt::{Display, Write as _};

use betterlite_logging::budget_email::event::{EntityType, WebhookEvent, WebhookEventType};
use betterlite_logging::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, get_budget_alert_type,
};
use rust_decimal::Decimal;
use secrecy::SecretString;

use super::CustomerSpend;
use super::thresholds::get_event_and_event_message;
use crate::failure_hook::ports::{BudgetAlertKind, CallInfo, EntityType as PortEntityType};

pub(super) struct BudgetAlertRequest {
    pub(super) event: WebhookEventType,
    pub(super) id: String,
    pub(super) message: String,
    pub(super) webhook_event: WebhookEvent,
}

#[must_use]
pub(super) const fn entity_type(event_group: PortEntityType) -> EntityType {
    match event_group {
        PortEntityType::Key => EntityType::Key,
        PortEntityType::User => EntityType::User,
        PortEntityType::EndUser => EntityType::EndUser,
        PortEntityType::Team => EntityType::Team,
        PortEntityType::TeamMember => EntityType::TeamMember,
        PortEntityType::Organization => EntityType::Organization,
        PortEntityType::OrganizationMember => EntityType::OrganizationMember,
        PortEntityType::Project => EntityType::Project,
        PortEntityType::Tag => EntityType::Tag,
        PortEntityType::Agent => EntityType::Agent,
        PortEntityType::ModelAccessGroup => EntityType::ModelAccessGroup,
        PortEntityType::Proxy => EntityType::Proxy,
    }
}

const fn alert_type_name(kind: BudgetAlertKind) -> &'static str {
    match kind {
        BudgetAlertKind::TokenBudget => "token_budget",
        BudgetAlertKind::UserBudget => "user_budget",
        BudgetAlertKind::SoftBudget => "soft_budget",
        BudgetAlertKind::MaxBudgetAlert => "max_budget_alert",
        BudgetAlertKind::TeamBudget => "team_budget",
        BudgetAlertKind::OrganizationBudget => "organization_budget",
        BudgetAlertKind::ProxyBudget => "proxy_budget",
        BudgetAlertKind::ProjectedLimitExceeded => "projected_limit_exceeded",
        BudgetAlertKind::ProjectBudget => "project_budget",
    }
}

pub(super) fn budget_alert(
    kind: BudgetAlertKind,
    user_info: &CallInfo,
) -> Option<BudgetAlertRequest> {
    let alert_type = get_budget_alert_type(alert_type_name(kind));
    let subject = BudgetAlertSubject {
        token: user_info.token.as_deref(),
        user_id: user_info.user_id.as_deref(),
        team_id: user_info.team_id.as_deref(),
        organization_id: user_info.organization_id.as_deref(),
        event_group: entity_type(user_info.event_group),
    };
    let id = alert_type.get_id(&subject);
    let user_info_str = get_user_info_str(user_info);
    let event_message = if user_info.event_group == PortEntityType::TeamMember {
        "Team Member Budget: ".to_owned()
    } else {
        alert_type.get_event_message().to_owned()
    };
    let projected_event = (kind == BudgetAlertKind::ProjectedLimitExceeded)
        .then_some(WebhookEventType::ProjectedLimitExceeded);
    if user_info.max_budget.is_none() && user_info.soft_budget.is_none() {
        return None;
    }
    let (event, resolved_message) =
        get_event_and_event_message(projected_event, user_info, event_message);
    let resolved_event = event?;
    let webhook_event = budget_webhook_event(user_info, resolved_event, resolved_message.clone());
    Some(BudgetAlertRequest {
        event: resolved_event,
        id,
        message: format!("{resolved_message}\n\n{user_info_str}"),
        webhook_event,
    })
}

#[must_use]
pub(super) fn budget_webhook_event(
    user_info: &CallInfo,
    event: WebhookEventType,
    event_message: String,
) -> WebhookEvent {
    WebhookEvent {
        event,
        event_message,
        event_group: entity_type(user_info.event_group),
        spend: user_info.spend,
        max_budget: user_info.max_budget,
        soft_budget: user_info.soft_budget,
        token: user_info.token.clone().map(SecretString::from),
        customer_id: user_info.customer_id.clone(),
        user_id: user_info.user_id.clone(),
        team_id: user_info.team_id.clone(),
        team_alias: user_info.team_alias.clone(),
        organization_id: user_info.organization_id.clone(),
        user_email: user_info.user_email.clone(),
        key_alias: user_info.key_alias.clone(),
        projected_exceeded_date: user_info.projected_exceeded_date.clone(),
        projected_spend: user_info.projected_spend,
        alert_emails: user_info.alert_emails.clone(),
        max_budget_alert_emails: user_info.max_budget_alert_emails.clone(),
    }
}

#[must_use]
pub(super) fn customer_spend_event(
    spend: &CustomerSpend<'_>,
    token: &str,
    end_user_id: &str,
    response_cost: Decimal,
) -> WebhookEvent {
    WebhookEvent {
        event: WebhookEventType::SpendTracked,
        event_message: format!(
            "Customer spend tracked. Customer={end_user_id}, spend={response_cost}"
        ),
        event_group: EntityType::EndUser,
        spend: response_cost,
        max_budget: spend.max_budget,
        soft_budget: None,
        token: Some(SecretString::from(token)),
        customer_id: Some(end_user_id.to_owned()),
        user_id: None,
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: None,
        key_alias: spend.key_alias.map(str::to_owned),
        projected_exceeded_date: None,
        projected_spend: None,
        alert_emails: None,
        max_budget_alert_emails: None,
    }
}

#[must_use]
pub(super) fn get_user_info_str(user_info: &CallInfo) -> String {
    let organization_id = user_info.organization_id.as_deref();
    let projected_date = user_info.projected_exceeded_date.as_deref();
    let event_group = entity_type(user_info.event_group);
    let alert_emails = user_info.alert_emails.as_ref();
    let max_budget_emails = user_info.max_budget_alert_emails.as_ref();
    let mut text = String::new();
    field(&mut text, "spend", Some(user_info.spend));
    field(&mut text, "max_budget", user_info.max_budget);
    field(&mut text, "soft_budget", user_info.soft_budget);
    field(&mut text, "customer_id", user_info.customer_id.as_deref());
    field(&mut text, "user_id", user_info.user_id.as_deref());
    field(&mut text, "team_id", user_info.team_id.as_deref());
    field(&mut text, "team_alias", user_info.team_alias.as_deref());
    field(&mut text, "organization_id", organization_id);
    field(&mut text, "user_email", user_info.user_email.as_deref());
    field(&mut text, "key_alias", user_info.key_alias.as_deref());
    field(&mut text, "projected_exceeded_date", projected_date);
    field(&mut text, "projected_spend", user_info.projected_spend);
    field(&mut text, "event_group", Some(event_group));
    field(
        &mut text,
        "alert_emails",
        alert_emails.map(|emails| format!("{emails:?}")),
    );
    field(
        &mut text,
        "max_budget_alert_emails",
        max_budget_emails.map(|entries| format!("{entries:?}")),
    );
    text
}

fn field<T>(text: &mut String, name: &str, value: Option<T>)
where
    T: Display,
{
    if let Some(displayable) = value {
        let _ignored = writeln!(text, "*{name}:* `{displayable}`");
    }
}
