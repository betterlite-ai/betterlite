use betterlite_logging::alert_settings::{SPEND_THRESHOLD_5_PERCENT, SPEND_THRESHOLD_15_PERCENT};
use betterlite_logging::budget_email::event::WebhookEventType;
use rust_decimal::Decimal;

use crate::failure_hook::ports::CallInfo;

#[must_use]
pub(super) fn get_percent_of_max_budget_left(user_info: &CallInfo) -> Decimal {
    let Some(max_budget) = user_info.max_budget else {
        return Decimal::ZERO;
    };
    if max_budget <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    max_budget
        .checked_sub(user_info.spend)
        .and_then(|left| left.checked_div(max_budget))
        .unwrap_or(Decimal::ZERO)
}

#[must_use]
pub(super) fn get_event_and_event_message(
    mut event: Option<WebhookEventType>,
    user_info: &CallInfo,
    mut event_message: String,
) -> (Option<WebhookEventType>, String) {
    let percent_left = get_percent_of_max_budget_left(user_info);
    let mut suffixes: Vec<String> = Vec::new();
    if let Some(soft_budget) = user_info.soft_budget
        && user_info.spend >= soft_budget
    {
        event = Some(WebhookEventType::SoftBudgetCrossed);
        suffixes.push(format!("Total Soft Budget:`{soft_budget}`"));
    }
    let max_outcome = match user_info.max_budget {
        Some(max_budget) if user_info.spend >= max_budget => Some((
            WebhookEventType::BudgetCrossed,
            format!("Budget Crossed\n Total Budget:`{max_budget}`"),
        )),
        Some(_) if percent_left <= SPEND_THRESHOLD_5_PERCENT => Some((
            WebhookEventType::ThresholdCrossed,
            "5% or less of budget remaining".to_owned(),
        )),
        Some(_) if percent_left <= SPEND_THRESHOLD_15_PERCENT => Some((
            WebhookEventType::ThresholdCrossed,
            "15% or less of budget remaining".to_owned(),
        )),
        Some(_) | None => None,
    };
    if let Some((max_event, max_suffix)) = max_outcome {
        event = Some(max_event);
        suffixes.push(max_suffix);
    }
    event_message.push_str(&suffixes.concat());
    (event, event_message)
}
