use crate::budget_email::event::EntityType;

#[path = "budget_alert_types/tests/factory.rs"]
mod factory;
#[path = "budget_alert_types/tests/org_budget.rs"]
mod org_budget;
#[path = "budget_alert_types/tests/project_budget.rs"]
mod project_budget;
#[path = "budget_alert_types/tests/projected.rs"]
mod projected;
#[path = "budget_alert_types/tests/soft_budget.rs"]
mod soft_budget;
#[path = "budget_alert_types/tests/team_budget.rs"]
mod team_budget;
#[path = "budget_alert_types/tests/token_budget.rs"]
mod token_budget;
#[path = "budget_alert_types/tests/user_budget.rs"]
mod user_budget;

// LiteLLM's get_id reads only these CallInfo fields, so the contract names them instead of the whole record
pub struct BudgetAlertSubject<'info> {
    pub token: Option<&'info str>,
    pub user_id: Option<&'info str>,
    pub team_id: Option<&'info str>,
    pub organization_id: Option<&'info str>,
    pub event_group: EntityType,
}

pub trait BaseBudgetAlertType {
    fn get_event_message(&self) -> &'static str;

    fn get_id(&self, user_info: &BudgetAlertSubject<'_>) -> String;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetAlertType {
    ProxyBudget,
    SoftBudget,
    UserBudget,
    TeamBudget,
    OrganizationBudget,
    TokenBudget,
    ProjectedLimitExceeded,
    ProjectBudget,
}

impl BaseBudgetAlertType for BudgetAlertType {
    fn get_event_message(&self) -> &'static str {
        match self {
            Self::ProxyBudget => "Proxy Budget: ",
            Self::SoftBudget => "Soft Budget Crossed: ",
            Self::UserBudget => "User Budget: ",
            Self::TeamBudget => "Team Budget: ",
            Self::OrganizationBudget => "Organization Budget: ",
            Self::TokenBudget => "Key Budget: ",
            Self::ProjectedLimitExceeded => "Key Budget: Projected Limit Exceeded",
            Self::ProjectBudget => "Project Budget: ",
        }
    }

    fn get_id(&self, user_info: &BudgetAlertSubject<'_>) -> String {
        match self {
            Self::ProxyBudget => "default_id".to_owned(),
            Self::SoftBudget if user_info.event_group == EntityType::Team => {
                user_info.team_id.unwrap_or("default_id").to_owned()
            }
            Self::SoftBudget => user_info.token.unwrap_or("default_id").to_owned(),
            Self::UserBudget => user_info.user_id.unwrap_or("default_id").to_owned(),
            Self::TeamBudget => user_info.team_id.unwrap_or("default_id").to_owned(),
            Self::OrganizationBudget => {
                user_info.organization_id.unwrap_or("default_id").to_owned()
            }
            Self::TokenBudget if user_info.event_group == EntityType::TeamMember => format!(
                "team_member:{}:{}",
                user_info.user_id.unwrap_or_default(),
                user_info.team_id.unwrap_or_default()
            ),
            Self::TokenBudget | Self::ProjectedLimitExceeded | Self::ProjectBudget => {
                user_info.token.unwrap_or("default_id").to_owned()
            }
        }
    }
}

#[must_use]
pub fn get_budget_alert_type(name: &str) -> BudgetAlertType {
    match name {
        "proxy_budget" => BudgetAlertType::ProxyBudget,
        "soft_budget" => BudgetAlertType::SoftBudget,
        "user_budget" => BudgetAlertType::UserBudget,
        "max_budget_alert" | "token_budget" => BudgetAlertType::TokenBudget,
        "team_budget" => BudgetAlertType::TeamBudget,
        "organization_budget" => BudgetAlertType::OrganizationBudget,
        "projected_limit_exceeded" => BudgetAlertType::ProjectedLimitExceeded,
        "project_budget" => BudgetAlertType::ProjectBudget,
        _ => BudgetAlertType::ProxyBudget,
    }
}
