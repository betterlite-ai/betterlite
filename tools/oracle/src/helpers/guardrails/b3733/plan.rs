use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_guardrails::model_armor::file_scanning::{ByteDataType, plan_file_scans};
use betterlite_prompts::message_content::Message;
use serde::Serialize;

use crate::helpers::{HelperResult, OrHelperError as _};

#[derive(Serialize)]
struct Attachment {
    byte_data_type: ByteDataType,
    file_bytes: String,
}

#[derive(Serialize)]
struct Plan {
    attachments: Vec<Attachment>,
    unscannable_count: usize,
}

pub(crate) fn plan(messages: &[Message]) -> HelperResult {
    let planned = plan_file_scans(messages);
    let attachments: Vec<Attachment> = planned
        .attachments
        .iter()
        .map(|attachment| Attachment {
            byte_data_type: attachment.byte_data_type,
            file_bytes: STANDARD.encode(&attachment.file_bytes),
        })
        .collect();
    serde_json::to_value(Plan {
        attachments,
        unscannable_count: planned.unscannable_count,
    })
    .or_value_error()
}
