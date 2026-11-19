use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::Number;

use crate::bedrock_records::{
    ChecksPiiEntry, ChecksResponse, ChecksScoreEntry, ChecksScoreResult, ChecksSensitiveResult,
    ChecksViolation,
};
use crate::guardrail_records::record::{GuardrailStatus, GuardrailTracingDetail};

const CONTENT_FILTER: &str = "contentFilter";
const PROMPT_ATTACK: &str = "promptAttack";
const SENSITIVE_INFORMATION: &str = "sensitiveInformation";

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ChecksThresholds {
    pub content_filter: Option<ConfidenceThreshold>,
    pub prompt_attack: Option<ConfidenceThreshold>,
    pub sensitive_information: Option<ConfidenceThreshold>,
}

// LiteLLM's _collect_invoke_checks_violations: only a configured check with a threshold can violate
#[must_use]
pub fn collect(
    response: Option<&ChecksResponse>,
    configured: &BTreeMap<String, GuardrailValue>,
    thresholds: ChecksThresholds,
) -> Vec<ChecksViolation> {
    let results = response.and_then(|value| value.results.as_ref());
    let mut violations = Vec::new();
    if configured.contains_key(CONTENT_FILTER) {
        collect_scores(
            CONTENT_FILTER,
            results.and_then(|value| value.content_filter.as_ref()),
            thresholds.content_filter,
            &mut violations,
        );
    }
    if configured.contains_key(PROMPT_ATTACK) {
        collect_scores(
            PROMPT_ATTACK,
            results.and_then(|value| value.prompt_attack.as_ref()),
            thresholds.prompt_attack,
            &mut violations,
        );
    }
    if configured.contains_key(SENSITIVE_INFORMATION) {
        collect_sensitive(
            SENSITIVE_INFORMATION,
            results.and_then(|value| value.sensitive_information.as_ref()),
            thresholds.sensitive_information,
            &mut violations,
        );
    }
    violations
}

fn collect_scores(
    check: &str,
    result: Option<&ChecksScoreResult>,
    threshold: Option<ConfidenceThreshold>,
    violations: &mut Vec<ChecksViolation>,
) {
    let Some(threshold) = threshold else {
        return;
    };
    let Some(entries) = result.and_then(|value| value.results.as_ref()) else {
        return;
    };
    for entry in entries {
        if let Some(violation) = score_violation(check, entry, threshold) {
            violations.push(violation);
        }
    }
}

fn score_violation(
    check: &str,
    entry: &ChecksScoreEntry,
    threshold: ConfidenceThreshold,
) -> Option<ChecksViolation> {
    let score = entry.severity_score.as_ref().and_then(Number::as_f64)?;
    if score < threshold.0 {
        return None;
    }
    Some(ChecksViolation {
        check: Some(check.to_owned()),
        category: entry.category.clone(),
        type_name: None,
        severity_score: Some(Number::from_f64(score)?),
        confidence_score: None,
        truncated: None,
        extra: BTreeMap::new(),
    })
}

// a truncated result counts as a violation: the detections it dropped were never scored
fn collect_sensitive(
    check: &str,
    result: Option<&ChecksSensitiveResult>,
    threshold: Option<ConfidenceThreshold>,
    violations: &mut Vec<ChecksViolation>,
) {
    let Some(threshold) = threshold else {
        return;
    };
    let Some(result) = result else {
        return;
    };
    if result.truncated == Some(true) {
        violations.push(ChecksViolation {
            check: Some(check.to_owned()),
            category: None,
            type_name: None,
            severity_score: None,
            confidence_score: None,
            truncated: Some(true),
            extra: BTreeMap::new(),
        });
    }
    let Some(entries) = result.results.as_ref() else {
        return;
    };
    for entry in entries {
        if let Some(violation) = pii_violation(check, entry, threshold) {
            violations.push(violation);
        }
    }
}

fn pii_violation(
    check: &str,
    entry: &ChecksPiiEntry,
    threshold: ConfidenceThreshold,
) -> Option<ChecksViolation> {
    let score = entry.confidence_score.as_ref().and_then(Number::as_f64)?;
    if score < threshold.0 {
        return None;
    }
    Some(ChecksViolation {
        check: Some(check.to_owned()),
        category: None,
        type_name: entry.type_name.clone(),
        severity_score: None,
        confidence_score: Some(Number::from_f64(score)?),
        truncated: None,
        extra: BTreeMap::new(),
    })
}

// LiteLLM's _get_invoke_checks_status
#[must_use]
pub const fn status(over_threshold: bool) -> GuardrailStatus {
    if over_threshold {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::Success
    }
}

// LiteLLM's _build_invoke_checks_tracing_detail
#[must_use]
pub fn tracing_detail(violations: &[ChecksViolation]) -> GuardrailTracingDetail {
    let categories: Vec<String> = violations
        .iter()
        .filter_map(label)
        .map(str::to_owned)
        .collect();
    GuardrailTracingDetail {
        guardrail_id: None,
        policy_template: None,
        detection_method: None,
        confidence_score: None,
        classification: None,
        match_details: None,
        patterns_checked: None,
        alert_recipients: None,
        risk_score: None,
        violation_categories: (!categories.is_empty()).then_some(categories),
        guardrail_action: Some(action(violations).to_owned()),
        guardrail_usage: None,
        guardrail_cost: None,
        guardrail_cost_by_unit: None,
        guardrail_cost_in_spend: None,
    }
}

fn label(violation: &ChecksViolation) -> Option<&str> {
    violation
        .category
        .as_deref()
        .filter(|value| !value.is_empty())
        .or(violation
            .type_name
            .as_deref()
            .filter(|value| !value.is_empty()))
}

const fn action(violations: &[ChecksViolation]) -> &'static str {
    if violations.is_empty() {
        "NONE"
    } else {
        "GUARDRAIL_INTERVENED"
    }
}
