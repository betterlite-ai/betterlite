use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::call_helpers::metadata::{
    MetadataMap, MetadataSlots, get_or_create_metadata_bucket,
};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::model_armor::decisions::{
    ModelArmorAPIError, append_armor_response, build_block_error_detail, build_logging_response,
    raise_if_fail_closed, should_block_content,
};
use crate::model_armor::file_scanning::{
    FileAttachment, FileScanPlan, MAX_FILE_SIZE_BYTES, plan_file_scans,
};
use crate::model_armor::transport::error::TransportError;
use crate::model_armor::transport::{ArmorCallError, ArmorResponse, ModelArmorTransport, Source};

const STATUS_KEY: &str = "_model_armor_status";
const RESPONSE_KEY: &str = "_model_armor_response";
const APPLIED_KEY: &str = "applied_guardrails";
const BLOCKED: &str = "blocked";
const SUCCESS: &str = "success";

#[derive(Debug, Clone)]
pub struct FileScanSettings {
    pub guardrail_name: Option<String>,
    pub fail_on_error: bool,
    pub skip_unscannable_attachments: bool,
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("Model Armor blocked the request")]
    Blocked {
        status: u16,
        detail: IndexMap<String, GuardrailValue>,
    },
    #[error(transparent)]
    Api(#[from] ModelArmorAPIError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

pub async fn scan_request_files(
    messages: &[Message],
    request_data: &mut MetadataSlots,
    transport: &ModelArmorTransport,
    settings: &FileScanSettings,
) -> Result<(), ScanError> {
    let plan = plan_file_scans(messages);
    let unscannable = unscannable_references(&plan, settings.skip_unscannable_attachments);
    if plan.attachments.is_empty() && unscannable == 0 {
        return Ok(());
    }
    let (_, metadata) = get_or_create_metadata_bucket(request_data);
    if let Some(name) = settings.guardrail_name.as_deref() {
        record_applied_guardrail(metadata, name);
    }
    check_unscannable(metadata, unscannable, settings.fail_on_error)?;
    for attachment in &plan.attachments {
        scan_attachment(metadata, transport, settings, attachment).await?;
    }
    Ok(())
}

fn unscannable_references(plan: &FileScanPlan, skip: bool) -> usize {
    if !skip {
        return plan.unscannable_count;
    }
    if plan.unscannable_count > 0 {
        tracing::warn!(
            count = plan.unscannable_count,
            "Model Armor: allowing unscannable attachments through because skip_unscannable_attachments is enabled"
        );
    }
    0
}

fn check_unscannable(
    metadata: &mut MetadataMap,
    count: usize,
    fail_on_error: bool,
) -> Result<(), ScanError> {
    if count == 0 {
        return Ok(());
    }
    let reason = format!(
        "{count} attachment(s) reference a document with no inline bytes (file_id or remote URL) that Model Armor cannot scan"
    );
    tracing::warn!(reason = %reason, "Model Armor: unscannable attachment");
    if !fail_on_error {
        return Ok(());
    }
    metadata.insert(STATUS_KEY.to_owned(), Json::Text(BLOCKED.to_owned()));
    Err(unscannable_block_error(&reason))
}

async fn scan_attachment(
    metadata: &mut MetadataMap,
    transport: &ModelArmorTransport,
    settings: &FileScanSettings,
    attachment: &FileAttachment,
) -> Result<(), ScanError> {
    let length = attachment.file_bytes.len();
    if length > MAX_FILE_SIZE_BYTES {
        return oversize(metadata, length, settings.fail_on_error);
    }
    let response = match transport
        .make_model_armor_request(
            None,
            Source::UserPrompt,
            Some((&attachment.file_bytes, attachment.byte_data_type)),
        )
        .await
    {
        Ok(response) => response,
        Err(ArmorCallError::Api(error)) => {
            raise_if_fail_closed(error, settings.fail_on_error)?;
            return Ok(());
        }
        Err(ArmorCallError::Transport(error)) => {
            tracing::error!(error = %error, "Model Armor file scan error");
            if settings.fail_on_error {
                return Err(ScanError::Transport(error));
            }
            return Ok(());
        }
    };
    record_response(metadata, transport, &response)
}

fn oversize(
    metadata: &mut MetadataMap,
    length: usize,
    fail_on_error: bool,
) -> Result<(), ScanError> {
    let reason = format!(
        "attachment of {length} bytes exceeds Model Armor's {MAX_FILE_SIZE_BYTES} byte scan limit"
    );
    tracing::warn!(reason = %reason, "Model Armor: attachment over the scan limit");
    if !fail_on_error {
        return Ok(());
    }
    metadata.insert(STATUS_KEY.to_owned(), Json::Text(BLOCKED.to_owned()));
    Err(unscannable_block_error(&reason))
}

fn record_response(
    metadata: &mut MetadataMap,
    transport: &ModelArmorTransport,
    response: &ArmorResponse,
) -> Result<(), ScanError> {
    let blocked = should_block_content(&response.verdict, false);
    let existing = metadata.get(RESPONSE_KEY).map(json_to_guardrail_value);
    let logged = build_logging_response(&response.body, transport.sanitize_error_detail);
    let appended = append_armor_response(existing, logged);
    metadata.insert(RESPONSE_KEY.to_owned(), guardrail_value_to_json(&appended));
    let sticky =
        matches!(metadata.get(STATUS_KEY), Some(Json::Text(current)) if current == BLOCKED);
    let status = if blocked || sticky { BLOCKED } else { SUCCESS };
    metadata.insert(STATUS_KEY.to_owned(), Json::Text(status.to_owned()));
    if !blocked {
        return Ok(());
    }
    Err(ScanError::Blocked {
        status: 400,
        detail: build_block_error_detail(
            "Content blocked by Model Armor",
            &response.body,
            transport.sanitize_error_detail,
        ),
    })
}

fn record_applied_guardrail(metadata: &mut MetadataMap, name: &str) {
    let entry = metadata
        .entry(APPLIED_KEY.to_owned())
        .or_insert_with(|| Json::List(Vec::new()));
    match entry {
        Json::List(names) => {
            if !names
                .iter()
                .any(|item| matches!(item, Json::Text(text) if text == name))
            {
                names.push(Json::Text(name.to_owned()));
            }
        }
        other => *other = Json::List(vec![Json::Text(name.to_owned())]),
    }
}

fn unscannable_block_error(reason: &str) -> ScanError {
    ScanError::Blocked {
        status: 400,
        detail: IndexMap::from([(
            "error".to_owned(),
            GuardrailValue::Text(format!(
                "Model Armor could not scan an attachment and blocked the request: {reason}"
            )),
        )]),
    }
}

fn json_to_guardrail_value(value: &Json) -> GuardrailValue {
    match value {
        Json::Null => GuardrailValue::Null,
        Json::Bool(flag) => GuardrailValue::Boolean(*flag),
        Json::Number(number) => number_to_guardrail_value(number),
        Json::Text(text) => GuardrailValue::Text(text.clone()),
        Json::List(items) => {
            GuardrailValue::Array(items.iter().map(json_to_guardrail_value).collect())
        }
        Json::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), json_to_guardrail_value(value)))
                .collect(),
        ),
    }
}

fn number_to_guardrail_value(number: &Number) -> GuardrailValue {
    if let Some(value) = number.as_i64() {
        return GuardrailValue::SignedInteger(value);
    }
    if let Some(value) = number.as_u64() {
        return GuardrailValue::UnsignedInteger(value);
    }
    if let Some(value) = number.as_f64() {
        return GuardrailValue::Decimal(value);
    }
    // GuardrailValue has no raw-number variant, so a number no Rust float holds keeps its literal text
    GuardrailValue::Text(number.to_string())
}

fn guardrail_value_to_json(value: &GuardrailValue) -> Json {
    match value {
        GuardrailValue::Null => Json::Null,
        GuardrailValue::Boolean(flag) => Json::Bool(*flag),
        GuardrailValue::SignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::UnsignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::Decimal(number) => {
            Number::from_f64(*number).map_or(Json::Null, Json::Number)
        }
        GuardrailValue::Text(text) => Json::Text(text.clone()),
        GuardrailValue::Array(items) => {
            Json::List(items.iter().map(guardrail_value_to_json).collect())
        }
        GuardrailValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), guardrail_value_to_json(value)))
                .collect(),
        ),
    }
}

mod tests;
