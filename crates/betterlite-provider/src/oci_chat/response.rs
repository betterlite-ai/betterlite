use std::collections::BTreeMap;

use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::oci_chat::config::get_vendor_from_model;
use crate::oci_chat_wire::cohere::reply::handle_cohere_response;
use crate::oci_chat_wire::generic::reply::handle_generic_response;
use crate::oci_chat_wire::types::oci::OciVendor;
use crate::oci_common::{Clock, OciError};

pub const MAX_CHAT_BODY_BYTES: usize = 0x0100_0000;

const INVALID_RESPONSE_FORMAT: &str = "Invalid response format from OCI";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciChatReply {
    pub response: ChatResponse,
    pub additional_headers: AdditionalHeaders,
}

pub fn decode_chat_body(status: u16, body: &[u8]) -> Result<Json, OciError> {
    if body.len() > MAX_CHAT_BODY_BYTES {
        return Err(OciError {
            status,
            message: format!(
                "the OCI chat response body is larger than {MAX_CHAT_BODY_BYTES} bytes"
            ),
        });
    }
    serde_json::from_slice(body).map_err(|source| OciError {
        status,
        message: format!("{INVALID_RESPONSE_FORMAT}: {source}"),
    })
}

pub fn transform_response(
    model: &str,
    status: u16,
    body: &[u8],
    headers: &IndexMap<String, String>,
    clock: &dyn Clock,
) -> Result<OciChatReply, OciError> {
    let document = decode_chat_body(status, body)?;
    let Json::Object(fields) = &document else {
        return Err(OciError {
            status,
            message: INVALID_RESPONSE_FORMAT.to_owned(),
        });
    };
    if let Some(error) = fields
        .get("error")
        .filter(|value| !matches!(value, Json::Null))
    {
        return Err(OciError {
            status,
            message: error_text(error),
        });
    }
    let mut response = empty_response();
    match get_vendor_from_model(model) {
        OciVendor::Cohere => {
            let created = u64::try_from(clock.now().timestamp()).unwrap_or_default();
            handle_cohere_response(&document, model, created, status, &mut response)?;
        }
        OciVendor::Generic => handle_generic_response(&document, status, &mut response)?,
    }
    Ok(OciChatReply {
        response,
        additional_headers: process_response_headers(HeaderSource::Upstream(headers)),
    })
}

fn empty_response() -> ChatResponse {
    ChatResponse {
        id: format!("chatcmpl-{}", Uuid::new_v4()),
        object: String::from("chat.completion"),
        created: 0,
        model: String::new(),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: None,
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: None,
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn error_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}
