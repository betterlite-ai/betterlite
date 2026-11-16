use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use uuid::Uuid;

use super::data::{CALL_ID, Data, LITELLM_METADATA};
use super::error::HookDataError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanType {
    Pre,
    Post,
    Streaming,
}

impl ScanType {
    const fn label(self) -> &'static str {
        match self {
            Self::Pre => "pre",
            Self::Post => "post",
            Self::Streaming => "streaming",
        }
    }
}

pub fn check_and_mark_scanned(
    data: &mut Data,
    scan_type: ScanType,
    new_id: fn() -> Uuid,
) -> Result<bool, HookDataError> {
    if super::data::call_id(data).is_none() {
        let id = new_id().to_string();
        tracing::warn!(
            "PANW Prisma AIRS: litellm_call_id missing from request data, synthesized {id} for {} scan deduplication",
            scan_type.label()
        );
        data.insert(CALL_ID.to_owned(), GuardrailValue::Text(id));
    }
    let call_id = super::data::call_id(data).unwrap_or_default();
    let key = format!("_panw_{}_scanned_{call_id}", scan_type.label());
    match data.get(LITELLM_METADATA) {
        Some(GuardrailValue::Object(bucket)) => match bucket.get(&key) {
            Some(GuardrailValue::Boolean(true)) => {
                tracing::debug!(
                    "PANW Prisma AIRS: Skipping duplicate {}-call scan",
                    scan_type.label()
                );
                return Ok(true);
            }
            None | Some(GuardrailValue::Null | GuardrailValue::Boolean(false)) => {}
            Some(_) => return Err(HookDataError::MarkerNotBoolean { key }),
        },
        None | Some(GuardrailValue::Null) => {}
        Some(_) => {
            return Err(HookDataError::NotObject {
                field: LITELLM_METADATA,
            });
        }
    }
    let mut bucket = match data.remove(LITELLM_METADATA) {
        Some(GuardrailValue::Object(fields)) => fields,
        None | Some(_) => BTreeMap::new(),
    };
    bucket.insert(key, GuardrailValue::Boolean(true));
    data.insert(LITELLM_METADATA.to_owned(), GuardrailValue::Object(bucket));
    Ok(false)
}
