use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::CompetitorIntentDetection;
use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::content_filter_records::overrides::{
    CompetitorIntentEvidenceEntry, CompetitorIntentResult,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;

use super::error::BLOCKED_STATUS;

pub const DEFAULT_REFUSE_MESSAGE: &str =
    "Content blocked: competitor comparison or ranking intent detected.";
pub const DEFAULT_REFRAME_MESSAGE: &str = concat!(
    "I can help with questions about our products and services. ",
    "Would you like to compare specific features or get more information?"
);

#[derive(Debug)]
pub struct CompetitorMessageTemplates<'templates> {
    pub refuse: Option<&'templates str>,
    pub reframe: Option<&'templates str>,
}

struct IntentSummary<'summary> {
    intent: &'summary str,
    confidence: f64,
}

pub fn apply_competitor_intent_policy(
    base: &GuardrailBase,
    result: &CompetitorIntentResult,
    templates: &CompetitorMessageTemplates<'_>,
    request_data: &BTreeMap<String, GuardrailValue>,
    detections: &mut Vec<ContentFilterDetection>,
) -> Result<(), GuardrailFailure> {
    let intent = result.intent.as_deref().unwrap_or("other");
    let confidence = result.confidence.unwrap_or_default();
    let action_hint = result.action_hint.as_deref().unwrap_or("allow");
    let detection = CompetitorIntentDetection {
        r#type: "competitor_intent".to_owned(),
        intent: intent.to_owned(),
        confidence,
        action_hint: action_hint.to_owned(),
        entities: result.entities.clone(),
        signals: result.signals.clone(),
        evidence: result.evidence.clone(),
    };
    detections.push(ContentFilterDetection::CompetitorIntent(detection));
    let summary = IntentSummary { intent, confidence };
    if action_hint == "refuse" {
        return Err(refuse_failure(templates, &summary));
    }
    if action_hint == "reframe" {
        return Err(reframe_failure(
            base,
            result,
            templates,
            request_data,
            &summary,
        ));
    }
    tracing::debug!(
        intent = %intent,
        confidence,
        action_hint = %action_hint,
        "content filter competitor intent continues"
    );
    Ok(())
}

fn refuse_failure(
    templates: &CompetitorMessageTemplates<'_>,
    summary: &IntentSummary<'_>,
) -> GuardrailFailure {
    let message = templates
        .refuse
        .filter(|text| !text.is_empty())
        .unwrap_or(DEFAULT_REFUSE_MESSAGE);
    tracing::warn!(
        intent = %summary.intent,
        confidence = summary.confidence,
        action_hint = %"refuse",
        "content filter competitor intent refuse"
    );
    let detail = IndexMap::from([
        ("error".to_owned(), GuardrailValue::Text(message.to_owned())),
        (
            "intent".to_owned(),
            GuardrailValue::Text(summary.intent.to_owned()),
        ),
        (
            "confidence".to_owned(),
            GuardrailValue::Decimal(summary.confidence),
        ),
    ]);
    GuardrailFailure::HttpObject {
        status: BLOCKED_STATUS,
        detail,
    }
}

fn reframe_failure(
    base: &GuardrailBase,
    result: &CompetitorIntentResult,
    templates: &CompetitorMessageTemplates<'_>,
    request_data: &BTreeMap<String, GuardrailValue>,
    summary: &IntentSummary<'_>,
) -> GuardrailFailure {
    let message = templates
        .reframe
        .filter(|text| !text.is_empty())
        .unwrap_or(DEFAULT_REFRAME_MESSAGE);
    tracing::info!(
        intent = %summary.intent,
        confidence = summary.confidence,
        action_hint = %"reframe",
        "content filter competitor intent reframe"
    );
    base.passthrough_failure(message, request_data, Some(result_info(result)), None)
}

// LiteLLM passes dict(intent_result), which leaves out a key the result does not carry
fn result_info(result: &CompetitorIntentResult) -> BTreeMap<String, GuardrailValue> {
    let evidence = result.evidence.iter().map(evidence_value).collect();
    let signals = result
        .signals
        .iter()
        .cloned()
        .map(GuardrailValue::Text)
        .collect();
    let mut info = BTreeMap::new();
    if let Some(intent) = &result.intent {
        info.insert("intent".to_owned(), GuardrailValue::Text(intent.clone()));
    }
    if let Some(confidence) = result.confidence {
        info.insert("confidence".to_owned(), GuardrailValue::Decimal(confidence));
    }
    info.insert("entities".to_owned(), entities_value(&result.entities));
    info.insert("signals".to_owned(), GuardrailValue::Array(signals));
    if let Some(action_hint) = &result.action_hint {
        info.insert(
            "action_hint".to_owned(),
            GuardrailValue::Text(action_hint.clone()),
        );
    }
    info.insert("evidence".to_owned(), GuardrailValue::Array(evidence));
    info
}

fn entities_value(entities: &BTreeMap<String, Vec<String>>) -> GuardrailValue {
    let mapped = entities
        .iter()
        .map(|(name, values)| {
            let items = values.iter().cloned().map(GuardrailValue::Text).collect();
            (name.clone(), GuardrailValue::Array(items))
        })
        .collect();
    GuardrailValue::Object(mapped)
}

// dict(entry) keeps only the keys the checker set, so a signal entry carries no value key
fn evidence_value(entry: &CompetitorIntentEvidenceEntry) -> GuardrailValue {
    let fields = [
        ("type", &entry.r#type),
        ("key", &entry.key),
        ("value", &entry.value),
        ("match", &entry.r#match),
    ];
    let mapped = fields
        .into_iter()
        .filter_map(|(name, value)| {
            value
                .clone()
                .map(|text| (name.to_owned(), GuardrailValue::Text(text)))
        })
        .collect();
    GuardrailValue::Object(mapped)
}
