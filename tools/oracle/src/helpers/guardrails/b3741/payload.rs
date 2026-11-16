use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::agent_365_tokens::payload::{EvaluatePayload, build_evaluate_payload};
use serde_json::json;

use super::{Data, PayloadAssemblyArgs, UUID_MARKER};
use crate::helpers::{HelperResult, OrHelperError as _};

fn insert_metadata(data: &mut Data, name: &str, metadata: Option<&Data>) {
    if let Some(metadata) = metadata {
        let _previous = data.insert(name.to_owned(), GuardrailValue::Object(metadata.clone()));
    }
}

fn insert_text(data: &mut Data, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        let _previous = data.insert(name.to_owned(), GuardrailValue::Text(value.to_owned()));
    }
}

fn request_data(args: &PayloadAssemblyArgs) -> Data {
    let mut data: Data = BTreeMap::new();
    let _previous = data.insert(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text(args.litellm_call_id.clone()),
    );
    if let Some(session) = &args.mcp_session_id {
        let metadata = BTreeMap::from([(
            "mcp_session_id".to_owned(),
            GuardrailValue::Text(session.clone()),
        )]);
        insert_metadata(&mut data, "mcp_tool_call_metadata", Some(&metadata));
    }
    insert_metadata(&mut data, "metadata", args.metadata.as_ref());
    let litellm_metadata = args.litellm_metadata.as_ref();
    insert_metadata(&mut data, "litellm_metadata", litellm_metadata);
    insert_text(&mut data, "mcp_tool_name", args.mcp_tool_name.as_deref());
    let server_name = args.mcp_server_name.as_deref();
    insert_text(&mut data, "mcp_server_name", server_name);
    insert_metadata(&mut data, "mcp_arguments", args.mcp_arguments.as_ref());
    data
}

pub(super) fn payload_result(args: &PayloadAssemblyArgs) -> HelperResult {
    let data = request_data(args);
    let payload = build_evaluate_payload(&data, args.key_alias.as_deref()).or_value_error()?;
    let generated = uuid::Uuid::parse_str(&payload.conversation_id)
        .is_ok_and(|parsed| parsed.get_version_num() == 4);
    let rendered = if generated {
        EvaluatePayload {
            conversation_id: UUID_MARKER.to_owned(),
            ..payload
        }
    } else {
        payload
    };
    Ok(json!({
        "payload": serde_json::to_value(&rendered).or_value_error()?,
        "conversation_id_is_uuid4": generated,
    }))
}
