#![cfg(test)]

use betterlite_core::api::responses_output_details::usage_details;
use betterlite_prompts::arguments::json::Object;
use uuid::Uuid;

use crate::gemini_live_accounting::tool_calls::ToolCallMessage;
use crate::gemini_live_client::records::ServerMessage;
use crate::gemini_live_client::state::{LiveSessionState, ToolCallNames};
use crate::gemini_live_tool_turn::turn::{
    ToolTurn, ToolTurnContext, ToolTurnInput, transform_tool_turn,
};

// every identifier repeats one byte, so the first drawn UUID shows the draw count
pub(crate) fn uuids() -> impl FnMut() -> Uuid {
    let mut drawn = 0_u128;
    move || {
        drawn = drawn.saturating_add(1);
        Uuid::from_u128(drawn.saturating_mul(0x0101_0101_0101_0101_0101_0101_0101_0101))
    }
}

pub(crate) fn empty_state() -> LiveSessionState {
    LiveSessionState {
        tool_call_names: ToolCallNames::default(),
        pending_usage_metadata: None,
        unbilled_input_audio_bytes: 0,
    }
}

pub(crate) fn given_ids() -> ToolTurnInput<'static> {
    ToolTurnInput {
        conversation_id: Some("conv_given"),
        response_id: Some("resp_given"),
        output_item_id: Some("item_given"),
        session_configuration_request: None,
    }
}

pub(crate) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("the object parses")
}

pub(crate) fn frame(source: &str) -> ServerMessage {
    serde_json::from_str(source).expect("the frame parses")
}

pub(crate) fn tool_call(source: &str) -> ToolCallMessage {
    serde_json::from_str(source).expect("the tool call parses")
}

pub(crate) fn run_turn(
    state: &mut LiveSessionState,
    frame: &mut ServerMessage,
    tool_call: &ToolCallMessage,
    input: &ToolTurnInput<'_>,
) -> ToolTurn {
    let mut context = ToolTurnContext {
        state,
        frame,
        tool_call,
        input,
    };
    transform_tool_turn(
        &mut context,
        &|usage| usage_details(Some(usage)),
        &mut uuids(),
    )
    .expect("the tool turn converts")
}
