use super::DOCS_URL;
use super::format::{format_percent, format_usd};
use super::report::CheckResult;
use super::settings::SpendCaptureProvider;

// The source's _alert_message: the alert a check outcome warrants, or None when the rate is healthy.
#[must_use]
pub fn alert_message(result: &CheckResult) -> Option<String> {
    match result {
        CheckResult::CredentialMissing { provider, env_var } => Some(format!(
            "Spend capture-rate check: {env_var} is not set, so the {} bill cannot be read. Set it or remove general_settings.spend_capture_rate_check. {DOCS_URL}",
            provider_name(*provider)
        )),
        CheckResult::RequestFailed { provider, detail } => Some(format!(
            "Spend capture-rate check: could not read the {} bill ({detail}). {DOCS_URL}",
            provider_name(*provider)
        )),
        CheckResult::Report(report) => {
            if !report.below_threshold {
                return None;
            }
            let rate = report.capture_rate?;
            Some(format!(
                "Spend capture rate for {} is {}, under the {} threshold: LiteLLM captured ${} of the ${} {} bill for {} to {}. Requests reach {} outside LiteLLM or cost tracking is dropping spend. {DOCS_URL}",
                provider_name(report.provider),
                format_percent(rate, 1),
                format_percent(report.threshold, 0),
                format_usd(report.captured_spend),
                format_usd(report.provider_spend),
                provider_name(report.provider),
                report.start_date,
                report.end_date,
                provider_name(report.provider),
            ))
        }
    }
}

const fn provider_name(provider: SpendCaptureProvider) -> &'static str {
    match provider {
        SpendCaptureProvider::Openai => "openai",
    }
}
