use crate::model_deprecation::alert::format_alert_message;
use crate::model_deprecation::types::{
    DeprecationStatus, ModelDeprecationInfo, ModelDeprecationResponse,
};

use super::support::{calendar_date, moment};

const ESCAPED_ALERT: &str = concat!(
    "*⚠️ Model Deprecation Warning*\n",
    "\n*Already deprecated:*\n",
    "• `a &amp; b &lt;c&gt; d` (provider: p &amp; &lt;q&gt;, deprecates 2026-04-01, ",
    "already deprecated 30d ago)\n",
    "\n*Deprecating within 30 days:*\n",
    "• `e &lt; f` (provider: unknown, deprecates 2026-05-10, in 9d)\n",
    "\nPlan migrations to a supported model. See ",
    "https://docs.litellm.ai/docs/proxy/model_management for guidance.",
);

const DEPRECATED_ONLY: &str = concat!(
    "*⚠️ Model Deprecation Warning*\n",
    "\n*Already deprecated:*\n",
    "• `alpha` (provider: openai, deprecates 2026-04-01, already deprecated 30d ago)\n",
    "\nPlan migrations to a supported model. See ",
    "https://docs.litellm.ai/docs/proxy/model_management for guidance.",
);

const IMMINENT_ONLY: &str = concat!(
    "*⚠️ Model Deprecation Warning*\n",
    "\n*Deprecating within 30 days:*\n",
    "• `alpha` (provider: azure, deprecates 2026-05-10, in 9d)\n",
    "• `beta` (provider: unknown, deprecates 2026-05-11, in 10d)\n",
    "• `gamma` (provider: openai, deprecates 2026-05-01, in 0d)\n",
    "\nPlan migrations to a supported model. See ",
    "https://docs.litellm.ai/docs/proxy/model_management for guidance.",
);

const UNKNOWN_PROVIDER: &str = concat!(
    "*⚠️ Model Deprecation Warning*\n",
    "\n*Deprecating within 30 days:*\n",
    "• `absent` (provider: unknown, deprecates 2026-05-10, in 9d)\n",
    "• `empty` (provider: unknown, deprecates 2026-05-10, in 9d)\n",
    "\nPlan migrations to a supported model. See ",
    "https://docs.litellm.ai/docs/proxy/model_management for guidance.",
);

fn info(name: &str, provider: Option<&str>, date: &str, days: i64) -> ModelDeprecationInfo {
    let status = if days < 0 {
        DeprecationStatus::Deprecated
    } else if days <= 30 {
        DeprecationStatus::Imminent
    } else {
        DeprecationStatus::Upcoming
    };
    ModelDeprecationInfo {
        model_name: name.to_owned(),
        litellm_model: Some(format!("raw/{name}")),
        deprecation_date: calendar_date(date),
        days_until_deprecation: days,
        status,
        litellm_provider: provider.map(str::to_owned),
    }
}

fn snapshot(
    deprecated: Vec<ModelDeprecationInfo>,
    imminent: Vec<ModelDeprecationInfo>,
    upcoming: Vec<ModelDeprecationInfo>,
) -> ModelDeprecationResponse {
    ModelDeprecationResponse {
        deprecated,
        imminent,
        upcoming,
        warn_within_days: 30,
        checked_at: moment("2026-05-01T12:00:00Z"),
    }
}

#[test]
fn escaped_alert() {
    let response = snapshot(
        vec![info("a & b <c> d", Some("p & <q>"), "2026-04-01", -30)],
        vec![info("e < f", None, "2026-05-10", 9)],
        Vec::new(),
    );
    let message = format_alert_message(&response).expect("a deprecated entry needs an alert");
    assert_eq!(message, ESCAPED_ALERT);
}

#[test]
fn deprecated_only_alert() {
    let response = snapshot(
        vec![info("alpha", Some("openai"), "2026-04-01", -30)],
        Vec::new(),
        Vec::new(),
    );
    let message = format_alert_message(&response).expect("a deprecated entry needs an alert");
    assert_eq!(message, DEPRECATED_ONLY);
}

#[test]
fn imminent_only_alert() {
    let response = snapshot(
        Vec::new(),
        vec![
            info("alpha", Some("azure"), "2026-05-10", 9),
            info("beta", None, "2026-05-11", 10),
            info("gamma", Some("openai"), "2026-05-01", 0),
        ],
        Vec::new(),
    );
    let message = format_alert_message(&response).expect("an imminent entry needs an alert");
    assert_eq!(message, IMMINENT_ONLY);
}

#[test]
fn unknown_provider_alert() {
    let response = snapshot(
        Vec::new(),
        vec![
            info("absent", None, "2026-05-10", 9),
            info("empty", Some(""), "2026-05-10", 9),
        ],
        Vec::new(),
    );
    let message = format_alert_message(&response).expect("an imminent entry needs an alert");
    assert_eq!(message, UNKNOWN_PROVIDER);
}

#[test]
fn upcoming_only_is_none() {
    let response = snapshot(
        Vec::new(),
        Vec::new(),
        vec![info("alpha", Some("openai"), "2026-09-01", 123)],
    );
    assert_eq!(format_alert_message(&response), None);
}

#[test]
fn empty_snapshot_is_none() {
    assert_eq!(
        format_alert_message(&snapshot(Vec::new(), Vec::new(), Vec::new())),
        None
    );
}
