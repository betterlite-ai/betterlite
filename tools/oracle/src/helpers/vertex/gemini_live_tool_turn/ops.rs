use betterlite_core::api::responses_output_details::usage_details;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::gemini_live_accounting::tool_calls::ToolCallMessage;
use betterlite_vertex::gemini_live_client::records::ServerMessage;
use betterlite_vertex::gemini_live_client::state::{LiveSessionState, ToolCallNames};
use betterlite_vertex::gemini_live_tool_turn::turn::{
    ToolTurnContext, ToolTurnInput, transform_tool_turn,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

// every identifier repeats the draw count in four 32-bit groups, the way the Python helper's counted uuid4 does
const UUID_PATTERN: u128 = 0x0000_0001_0000_0001_0000_0001_0000_0001;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    tool_call: ToolCallMessage,
    ids: Ids,
    #[serde(default)]
    frame: Option<ServerMessage>,
    #[serde(default)]
    session_configuration_request: Option<String>,
    #[serde(default)]
    pending_usage: Option<Object>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ids {
    #[serde(rename = "conversation_id", default)]
    conversation: Option<String>,
    #[serde(rename = "response_id", default)]
    response: Option<String>,
    #[serde(rename = "output_item_id", default)]
    output_item: Option<String>,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut state = LiveSessionState {
        tool_call_names: ToolCallNames::default(),
        pending_usage_metadata: input.pending_usage.clone(),
        unbilled_input_audio_bytes: 0,
    };
    let mut frame = input.frame.clone().unwrap_or_default();
    let mut drawn = 0_u128;
    let mut uuids = || {
        drawn = drawn.saturating_add(1);
        Uuid::from_u128(drawn.saturating_mul(UUID_PATTERN))
    };
    let turn_input = ToolTurnInput {
        conversation_id: input.ids.conversation.as_deref(),
        response_id: input.ids.response.as_deref(),
        output_item_id: input.ids.output_item.as_deref(),
        session_configuration_request: input.session_configuration_request.as_deref(),
    };
    let turn = {
        let mut context = ToolTurnContext {
            state: &mut state,
            frame: &mut frame,
            tool_call: &input.tool_call,
            input: &turn_input,
        };
        transform_tool_turn(
            &mut context,
            &|usage| usage_details(Some(usage)),
            &mut uuids,
        )
        .or_value_error()?
    };
    Ok(json!({
        "events": turn.events,
        "ids": {
            "conversation_id": turn.conversation_id,
            "response_id": turn.response_id,
            "output_item_id": turn.output_item_id,
        },
        "pending_usage": state.pending_usage_metadata,
    }))
}
