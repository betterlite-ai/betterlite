#![cfg(test)]

mod corpus;
mod decode_tools;
mod extract;
mod format;
mod messages;
mod passthrough;
mod responses_input;
mod responses_passthrough;
mod tool_shapes;
mod typed_failures;
mod update_tools;

use crate::arguments::json::Json;
use crate::file_ids::mapping::FileIdMapping;
use crate::message_content::Message;

const DEPLOYMENT: &str = "deployment-1";
const FILE_UNIFIED: &str = "bGl0ZWxsbV9wcm94eTpmaWxlLmFiYzEyMzt1bmlmaWVkX2lkLGFiYy0xMjM7dGFyZ2V0X21vZGVsX25hbWVzLGdwdC00O2xsbV9vdXRwdXRfZmlsZV9pZCxmaWxlLXh5ejtsbG1fb3V0cHV0X2ZpbGVfbW9kZWxfaWQsZGVwbG95bWVudC0x";
const FILE_UNIFIED_NO_OUTPUT: &str = "bGl0ZWxsbV9wcm94eTpmaWxlLmFiYzEyMzt1bmlmaWVkX2lkLGFiYy0xMjM";
const FILE_EMBEDDED: &str = "file-bGl0ZWxsbTpmaWxlLWFiYzEyMzttb2RlbCxncHQtNG8tbGl0ZWxsbQ";
const VECTOR_UNIFIED: &str = "bGl0ZWxsbV9wcm94eTp2ZWN0b3Jfc3RvcmU7dW5pZmllZF9pZCxhYmMtMTIzO3RhcmdldF9tb2RlbF9uYW1lcyxncHQtNDtyZXNvdXJjZV9pZCx2c19hYmMxMjM7bW9kZWxfaWQsZGVwbG95bWVudC0x";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("test json decodes")
}

fn mapping(text: &str) -> FileIdMapping {
    serde_json::from_str(text).expect("test mapping decodes")
}

fn messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).expect("test messages decode")
}

fn serialized<Payload>(payload: &Payload) -> Json
where
    Payload: serde::Serialize,
{
    json(&serde_json::to_string(payload).expect("test payload serializes"))
}
