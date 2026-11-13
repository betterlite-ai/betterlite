use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use betterlite_guardrails::lakera_pii::wire::BreakdownItem;
use betterlite_guardrails::lakera_v2::advisory::{
    humanize_lakera_block_reasons, validate_advisory_config,
};
use betterlite_guardrails::lakera_v2::scope::inspection_messages;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
struct HumanizedItem {
    #[serde(default)]
    detected: Option<bool>,
    #[serde(default)]
    detector_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    HumanizeLakeraBlockReasons {
        #[serde(default)]
        breakdown: Option<Vec<HumanizedItem>>,
    },
    ValidateAdvisoryConfig {
        #[serde(default)]
        on_flagged: Option<LakeraOnFlagged>,
        #[serde(default)]
        advisory_system_message: Option<String>,
        #[serde(default)]
        payload: Option<bool>,
        #[serde(default)]
        breakdown: Option<bool>,
    },
    BuildLakeraInspectionMessages {
        data: BTreeMap<String, GuardrailValue>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let value = match input {
            Input::HumanizeLakeraBlockReasons { breakdown } => {
                let items = breakdown_items(breakdown);
                json!({ "reason": humanize_lakera_block_reasons(Some(items.as_slice())) })
            }
            Input::ValidateAdvisoryConfig {
                on_flagged,
                advisory_system_message,
                payload,
                breakdown,
            } => {
                let on_flagged = on_flagged.unwrap_or(LakeraOnFlagged::Block);
                let verdict = validate_advisory_config(
                    on_flagged,
                    advisory_system_message.as_deref(),
                    *payload,
                    *breakdown,
                );
                match verdict {
                    Ok(()) => json!({ "ok": true }),
                    Err(error) => json!({ "ok": false, "error": error.to_string() }),
                }
            }
            Input::BuildLakeraInspectionMessages { data } => {
                json!({ "messages": inspection_messages(data) })
            }
        };
        Ok(value)
    }
}

fn breakdown_items(items: &Option<Vec<HumanizedItem>>) -> Vec<BreakdownItem> {
    items
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|item| BreakdownItem {
            detected: item.detected,
            detector_type: item.detector_type.clone(),
            extra: BTreeMap::new(),
        })
        .collect()
}
