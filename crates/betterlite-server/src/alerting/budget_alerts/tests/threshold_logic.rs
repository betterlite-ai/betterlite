#![cfg(test)]

use betterlite_logging::budget_email::event::WebhookEventType;

use super::support::{call_info, decimal};
use super::thresholds::get_event_and_event_message;

#[test]
fn a_soft_budget_crossing_sets_the_soft_event_and_text() {
    let mut info = call_info(decimal("50"));
    info.soft_budget = Some(decimal("50"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, Some(WebhookEventType::SoftBudgetCrossed));
    assert_eq!(message, "User Budget: Total Soft Budget:`50`");
}

#[test]
fn a_soft_budget_that_is_not_crossed_keeps_the_event() {
    let mut info = call_info(decimal("49.99"));
    info.soft_budget = Some(decimal("50"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, None);
    assert_eq!(message, "User Budget: ");
}

#[test]
fn a_max_budget_crossing_overrides_the_soft_event() {
    let mut info = call_info(decimal("120"));
    info.soft_budget = Some(decimal("50"));
    info.max_budget = Some(decimal("100"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, Some(WebhookEventType::BudgetCrossed));
    assert_eq!(
        message,
        "User Budget: Total Soft Budget:`50`Budget Crossed\n Total Budget:`100`"
    );
}

#[test]
fn a_crossed_soft_budget_keeps_its_event_when_the_max_budget_holds() {
    let mut info = call_info(decimal("60"));
    info.soft_budget = Some(decimal("50"));
    info.max_budget = Some(decimal("100"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, Some(WebhookEventType::SoftBudgetCrossed));
    assert_eq!(message, "User Budget: Total Soft Budget:`50`");
}

#[test]
fn exactly_five_percent_left_sets_the_five_percent_text() {
    let mut info = call_info(decimal("95"));
    info.max_budget = Some(decimal("100"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, Some(WebhookEventType::ThresholdCrossed));
    assert_eq!(message, "User Budget: 5% or less of budget remaining");
}

#[test]
fn exactly_fifteen_percent_left_sets_the_fifteen_percent_text() {
    let mut info = call_info(decimal("85"));
    info.max_budget = Some(decimal("100"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, Some(WebhookEventType::ThresholdCrossed));
    assert_eq!(message, "User Budget: 15% or less of budget remaining");
}

#[test]
fn more_than_fifteen_percent_left_keeps_the_projected_event() {
    let mut info = call_info(decimal("50"));
    info.max_budget = Some(decimal("100"));

    let (event, message) = get_event_and_event_message(
        Some(WebhookEventType::ProjectedLimitExceeded),
        &info,
        "Key Budget: Projected Limit Exceeded".to_owned(),
    );

    assert_eq!(event, Some(WebhookEventType::ProjectedLimitExceeded));
    assert_eq!(message, "Key Budget: Projected Limit Exceeded");
}

#[test]
fn no_budget_leaves_the_event_alone() {
    let info = call_info(decimal("50"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, None);
    assert_eq!(message, "User Budget: ");
}

#[test]
fn a_zero_max_budget_counts_as_crossed() {
    let mut info = call_info(decimal("0"));
    info.max_budget = Some(decimal("0"));

    let (event, message) = get_event_and_event_message(None, &info, "User Budget: ".to_owned());

    assert_eq!(event, Some(WebhookEventType::BudgetCrossed));
    assert_eq!(message, "User Budget: Budget Crossed\n Total Budget:`0`");
}
