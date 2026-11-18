use super::types::{ModelDeprecationInfo, ModelDeprecationResponse};

fn escape_slack_mrkdwn(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn format_entry(info: &ModelDeprecationInfo) -> String {
    let suffix = if info.days_until_deprecation < 0 {
        format!(
            "already deprecated {}d ago",
            info.days_until_deprecation.unsigned_abs()
        )
    } else {
        format!("in {}d", info.days_until_deprecation)
    };
    let provider = match &info.litellm_provider {
        Some(provider) if !provider.is_empty() => escape_slack_mrkdwn(provider),
        Some(_) | None => "unknown".to_owned(),
    };
    format!(
        "• `{}` (provider: {provider}, deprecates {}, {suffix})",
        escape_slack_mrkdwn(&info.model_name),
        info.deprecation_date
    )
}

#[must_use]
pub fn format_alert_message(snapshot: &ModelDeprecationResponse) -> Option<String> {
    if snapshot.deprecated.is_empty() && snapshot.imminent.is_empty() {
        return None;
    }
    let mut lines = vec!["*⚠️ Model Deprecation Warning*".to_owned()];
    if !snapshot.deprecated.is_empty() {
        lines.push("\n*Already deprecated:*".to_owned());
        lines.extend(snapshot.deprecated.iter().map(format_entry));
    }
    if !snapshot.imminent.is_empty() {
        lines.push(format!(
            "\n*Deprecating within {} days:*",
            snapshot.warn_within_days
        ));
        lines.extend(snapshot.imminent.iter().map(format_entry));
    }
    lines.push(
        "\nPlan migrations to a supported model. See https://docs.litellm.ai/docs/proxy/model_management for guidance."
            .to_owned(),
    );
    Some(lines.join("\n"))
}
