pub mod format;

use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;

use crate::lakera_pii::wire::BreakdownItem;

use super::error::LakeraV2Error;

const PLACEHOLDER: &str = "placeholder";

#[must_use]
pub fn humanize_lakera_block_reasons(breakdown: Option<&[BreakdownItem]>) -> String {
    let Some(items) = breakdown.filter(|items| !items.is_empty()) else {
        return "a content safety concern".to_owned();
    };
    let mut phrases: Vec<String> = Vec::new();
    for item in items {
        if item.detected != Some(true) {
            continue;
        }
        let detector_type = item.detector_type.as_deref().unwrap_or_default();
        let category = detector_type.split('/').next().unwrap_or_default();
        if category.is_empty() {
            continue;
        }
        let phrase = category_phrase(category);
        if !phrases.contains(&phrase) {
            phrases.push(phrase);
        }
    }
    if phrases.is_empty() {
        return "a content safety concern".to_owned();
    }
    phrases.join(", ")
}

fn category_phrase(category: &str) -> String {
    match category {
        "prompt_injection" | "prompt_attack" => "a potential prompt injection attempt".to_owned(),
        "pii" => "personally identifiable information".to_owned(),
        "moderated_content" => "policy-violating content".to_owned(),
        other => other.replace('_', " "),
    }
}

pub fn validate_advisory_config(
    on_flagged: LakeraOnFlagged,
    template: Option<&str>,
    payload: Option<bool>,
    breakdown: Option<bool>,
) -> Result<(), LakeraV2Error> {
    if on_flagged != LakeraOnFlagged::InjectSystemMessage {
        return Ok(());
    }
    if let Some(template) = template {
        let placeholder = format::uses_reason_placeholder(template)
            .map_err(|source| LakeraV2Error::AdvisorySyntax { source })?;
        if !placeholder {
            return Err(LakeraV2Error::AdvisoryPlaceholder);
        }
        format::format_reason(template, PLACEHOLDER)
            .map_err(|source| LakeraV2Error::AdvisoryFormat { source })?;
    }
    if payload != Some(true) || breakdown != Some(true) {
        return Err(LakeraV2Error::AdvisoryNeedsFlags);
    }
    Ok(())
}
