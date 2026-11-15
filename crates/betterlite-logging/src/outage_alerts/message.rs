use crate::alert_dispatch::AlertLevel;

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
pub enum OutageAlertType {
    Minor,
    Major,
}

impl OutageAlertType {
    #[must_use]
    pub const fn level(self) -> AlertLevel {
        match self {
            Self::Minor => AlertLevel::Medium,
            Self::Major => AlertLevel::High,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
pub enum OutageKey {
    Model,
    Region,
}

pub struct OutageMessage<'message> {
    pub alert_type: OutageAlertType,
    pub key: OutageKey,
    pub key_value: &'message str,
    pub provider: &'message str,
    pub api_base: Option<&'message str>,
    pub alerts: &'message [i64],
    pub now: f64,
    pub last_updated_at: f64,
}

#[must_use]
pub fn count_outage_alerts(alerts: &[i64]) -> String {
    let mut timeouts: usize = 0;
    let mut api_errors: usize = 0;
    let mut unknown_errors: usize = 0;
    for status in alerts {
        if *status == 408 {
            timeouts = timeouts.saturating_add(1);
        } else if *status >= 500 {
            api_errors = api_errors.saturating_add(1);
        } else {
            unknown_errors = unknown_errors.saturating_add(1);
        }
    }
    let buckets = [
        ("Timeout Errors", timeouts),
        ("API Errors", api_errors),
        ("Unknown Errors", unknown_errors),
    ];
    let mut parts = Vec::new();
    for (name, count) in buckets {
        if count > 0 {
            parts.push(format!("\n{name}: {count}\n"));
        }
    }
    parts.concat()
}

#[must_use]
pub fn outage_alert_message(message: &OutageMessage<'_>) -> String {
    let alert_type = message.alert_type;
    let key = message.key;
    let key_value = message.key_value;
    let provider = message.provider;
    let mut parts = vec![format!(
        "\n\n\n*\u{26a0}\u{fe0f} {alert_type} Service Outage*\n\n\n"
    )];
    parts.push(format!("*{key} Name:* `{key_value}`\n"));
    parts.push(format!("*Provider:* `{provider}`\n"));
    if let Some(api_base) = message.api_base {
        parts.push(format!("*API Base:* `{api_base}`\n"));
    }
    let errors = count_outage_alerts(message.alerts);
    parts.push(format!("\n\n*Errors:*\n{errors}\n\n"));
    let seconds = last_check_text(message.now, message.last_updated_at);
    parts.push(format!("*Last Check:* `{seconds}s ago`\n\n\n"));
    parts.concat()
}

// Python prints round(x, 4) through str(), which keeps a shortest form with at least one decimal digit.
fn last_check_text(now: f64, last_updated_at: f64) -> String {
    // float_arithmetic is denied; a fused multiply-add computes now - last_updated_at
    let elapsed = last_updated_at.mul_add(-1.0_f64, now);
    let fixed = format!("{elapsed:.4}");
    let Some((whole, fraction)) = fixed.split_once('.') else {
        return fixed;
    };
    let trimmed = fraction.trim_end_matches('0');
    if trimmed.is_empty() {
        return format!("{whole}.0");
    }
    format!("{whole}.{trimmed}")
}
