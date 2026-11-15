#![cfg(test)]

use rstest::rstest;

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType, get_budget_alert_type,
};

#[test]
fn the_proxy_budget_names_itself_and_uses_the_default_id() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: Some("org-1"),
        event_group: EntityType::Team,
    };

    assert_eq!(
        BudgetAlertType::ProxyBudget.get_event_message(),
        "Proxy Budget: "
    );
    assert_eq!(BudgetAlertType::ProxyBudget.get_id(&subject), "default_id");
}

#[rstest]
#[case("proxy_budget", BudgetAlertType::ProxyBudget)]
#[case("soft_budget", BudgetAlertType::SoftBudget)]
#[case("user_budget", BudgetAlertType::UserBudget)]
#[case("team_budget", BudgetAlertType::TeamBudget)]
#[case("organization_budget", BudgetAlertType::OrganizationBudget)]
#[case("token_budget", BudgetAlertType::TokenBudget)]
#[case("max_budget_alert", BudgetAlertType::TokenBudget)]
#[case("projected_limit_exceeded", BudgetAlertType::ProjectedLimitExceeded)]
#[case("project_budget", BudgetAlertType::ProjectBudget)]
fn each_known_name_builds_its_alert(#[case] name: &str, #[case] expected: BudgetAlertType) {
    assert_eq!(get_budget_alert_type(name), expected);
}

#[rstest]
#[case("user_budgets")]
#[case("Organization_Budget")]
#[case("")]
fn an_unknown_name_falls_back_to_the_proxy_budget(#[case] name: &str) {
    assert_eq!(
        get_budget_alert_type(name),
        BudgetAlertType::ProxyBudget,
        "LiteLLM builds a proxy budget alert for a name it does not know"
    );
}
