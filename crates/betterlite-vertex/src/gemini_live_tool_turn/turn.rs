use betterlite_models::usage_sum::Usage;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_prompts::arguments::json::Object;
use uuid::Uuid;

use super::error::ToolTurnError;
use super::events::{
    ArgumentsDeltaEvent, ConversationItemAddedEvent, FunctionCallItem, OutputItemEvent,
    ResponseCreatedEvent, ResponseCreatedObject, ToolTurnDoneEvent, ToolTurnDoneResponse,
    ToolTurnEvent,
};
use crate::gemini_live_accounting::aliases::add_usage_detail_aliases;
use crate::gemini_live_accounting::tool_calls::{
    FunctionCallArgumentsDone, ToolCallIds, ToolCallMessage, transform_tool_call_events,
};
use crate::gemini_live_accounting::usage::{chat_usage, consume_usage_metadata};
use crate::gemini_live_client::records::ServerMessage;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_setup::decode::decode_gemini_live_setup;
use crate::gemini_live_setup::wire::GenerationConfig;

#[derive(Clone, Copy, Debug, Default)]
pub struct ToolTurnInput<'input> {
    pub conversation_id: Option<&'input str>,
    pub response_id: Option<&'input str>,
    pub output_item_id: Option<&'input str>,
    pub session_configuration_request: Option<&'input str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolTurn {
    pub events: Vec<ToolTurnEvent>,
    pub conversation_id: Option<String>,
    pub response_id: Option<String>,
    pub output_item_id: Option<String>,
}

pub fn transform_tool_turn(
    state: &mut LiveSessionState,
    frame: &mut ServerMessage,
    tool_call: &ToolCallMessage,
    input: &ToolTurnInput<'_>,
    project_usage: &dyn Fn(&Usage) -> ResponseApiUsage,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<ToolTurn, ToolTurnError> {
    if tool_call.function_calls.is_empty() {
        return Ok(ToolTurn {
            events: Vec::new(),
            conversation_id: input.conversation_id.map(str::to_owned),
            response_id: input.response_id.map(str::to_owned),
            output_item_id: input.output_item_id.map(str::to_owned),
        });
    }
    let conversation_id = input
        .conversation_id
        .map_or_else(|| format!("conv_{}", uuids()), str::to_owned);
    let setup = generation_config(input.session_configuration_request);
    let modalities: Vec<String> = match setup.response_modalities.as_deref() {
        Some(configured) => configured
            .iter()
            .map(|modality| modality.to_lowercase())
            .collect(),
        None => vec!["audio".to_owned()],
    };
    let mut events = Vec::new();
    let (response_id, item_base) = match input.response_id {
        Some(existing) => (existing.to_owned(), input.output_item_id.map(str::to_owned)),
        None => {
            let response_id = format!("resp_{}", uuids());
            let item_base = Some(format!("item_{}", uuids()));
            let event_id = format!("event_{}", uuids());
            events.push(ToolTurnEvent::Created(ResponseCreatedEvent {
                event_type: "response.created",
                event_id,
                response: ResponseCreatedObject {
                    object: "realtime.response",
                    id: response_id.clone(),
                    status: "in_progress",
                    status_details: None,
                    output: Vec::new(),
                    conversation_id: conversation_id.clone(),
                    modalities: modalities.clone(),
                    temperature: setup.temperature.clone(),
                    max_output_tokens: setup.max_output_tokens.clone(),
                },
            }));
            (response_id, item_base)
        }
    };
    let calls = transform_tool_call_events(
        &mut state.tool_call_names,
        tool_call,
        ToolCallIds {
            response_id: Some(response_id.as_str()),
            output_item_id: item_base.as_deref(),
        },
        uuids,
    );
    for (index, call) in calls.iter().enumerate() {
        push_call_events(&mut events, call, index, &response_id, uuids);
    }
    let reported = consume_usage_metadata(state, frame);
    // the frame's grounding counters belong to no tool call, so this turn bills usage alone
    let chat = chat_usage(reported.as_ref(), None)?;
    let mut usage: Object = serde_json::from_slice(&serde_json::to_vec(&project_usage(&chat))?)?;
    add_usage_detail_aliases(&mut usage);
    events.push(ToolTurnEvent::Done(ToolTurnDoneEvent {
        event_type: "response.done",
        event_id: format!("event_{}", uuids()),
        response: ToolTurnDoneResponse {
            object: "realtime.response",
            id: response_id,
            status: "completed",
            status_details: None,
            output: calls.iter().map(completed_item).collect(),
            conversation_id: conversation_id.clone(),
            modalities,
            usage,
            temperature: setup.temperature,
            max_output_tokens: setup.max_output_tokens,
        },
    }));
    Ok(ToolTurn {
        events,
        conversation_id: Some(conversation_id),
        response_id: None,
        output_item_id: None,
    })
}

fn push_call_events(
    events: &mut Vec<ToolTurnEvent>,
    call: &FunctionCallArgumentsDone,
    index: usize,
    response_id: &str,
    uuids: &mut dyn FnMut() -> Uuid,
) {
    let completed = completed_item(call);
    let pending = FunctionCallItem {
        status: "in_progress",
        arguments: String::new(),
        ..completed.clone()
    };
    events.push(ToolTurnEvent::OutputItemAdded(OutputItemEvent {
        event_type: "response.output_item.added",
        event_id: format!("event_{}", uuids()),
        response_id: response_id.to_owned(),
        output_index: index,
        item: pending.clone(),
    }));
    events.push(ToolTurnEvent::ConversationItemAdded(
        ConversationItemAddedEvent {
            event_type: "conversation.item.added",
            event_id: format!("event_{}", uuids()),
            previous_item_id: None,
            item: pending,
        },
    ));
    events.push(ToolTurnEvent::ArgumentsDelta(ArgumentsDeltaEvent {
        event_type: "response.function_call_arguments.delta",
        event_id: format!("event_{}", uuids()),
        response_id: response_id.to_owned(),
        item_id: call.item_id.clone(),
        output_index: index,
        call_id: call.call_id.clone(),
        delta: call.arguments.clone(),
    }));
    events.push(ToolTurnEvent::ArgumentsDone(call.clone()));
    events.push(ToolTurnEvent::OutputItemDone(OutputItemEvent {
        event_type: "response.output_item.done",
        event_id: format!("event_{}", uuids()),
        response_id: response_id.to_owned(),
        output_index: index,
        item: completed,
    }));
}

fn completed_item(call: &FunctionCallArgumentsDone) -> FunctionCallItem {
    FunctionCallItem {
        id: call.item_id.clone(),
        object: "realtime.item",
        item_type: "function_call",
        status: "completed",
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
    }
}

fn generation_config(stored_setup: Option<&str>) -> GenerationConfig {
    stored_setup
        .and_then(|text| decode_gemini_live_setup(text).ok())
        .map_or_else(GenerationConfig::default, |setup| setup.generation_config)
}
