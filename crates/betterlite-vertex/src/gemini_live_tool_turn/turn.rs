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

pub struct ToolTurnContext<'turn> {
    pub state: &'turn mut LiveSessionState,
    pub frame: &'turn mut ServerMessage,
    pub tool_call: &'turn ToolCallMessage,
    pub input: &'turn ToolTurnInput<'turn>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolTurn {
    pub events: Vec<ToolTurnEvent>,
    pub conversation_id: Option<String>,
    pub response_id: Option<String>,
    pub output_item_id: Option<String>,
}

struct TurnSetup {
    conversation_id: String,
    modalities: Vec<String>,
    configuration: GenerationConfig,
}

impl TurnSetup {
    fn new(input: &ToolTurnInput<'_>, uuids: &mut dyn FnMut() -> Uuid) -> Self {
        let configuration = generation_config(input.session_configuration_request);
        let modalities = configuration.response_modalities.as_deref().map_or_else(
            || vec!["audio".to_owned()],
            |configured| {
                configured
                    .iter()
                    .map(|modality| modality.to_lowercase())
                    .collect()
            },
        );
        let conversation_id = input
            .conversation_id
            .map_or_else(|| format!("conv_{}", uuids()), str::to_owned);
        Self {
            conversation_id,
            modalities,
            configuration,
        }
    }
}

pub fn transform_tool_turn(
    context: &mut ToolTurnContext<'_>,
    project_usage: &dyn Fn(&Usage) -> ResponseApiUsage,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<ToolTurn, ToolTurnError> {
    if context.tool_call.function_calls.is_empty() {
        return Ok(ToolTurn {
            events: Vec::new(),
            conversation_id: context.input.conversation_id.map(str::to_owned),
            response_id: context.input.response_id.map(str::to_owned),
            output_item_id: context.input.output_item_id.map(str::to_owned),
        });
    }
    let setup = TurnSetup::new(context.input, uuids);
    let mut events = Vec::new();
    let (response_id, item_base) = response_base(context.input, &setup, &mut events, uuids);
    let calls = transform_tool_call_events(
        &mut context.state.tool_call_names,
        context.tool_call,
        ToolCallIds {
            response_id: Some(response_id.as_str()),
            output_item_id: item_base.as_deref(),
        },
        uuids,
    );
    for (index, call) in calls.iter().enumerate() {
        push_call_events(&mut events, call, index, &response_id, uuids);
    }
    let usage = turn_usage(context, project_usage)?;
    events.push(done_event(&setup, &response_id, &calls, usage, uuids));
    Ok(ToolTurn {
        events,
        conversation_id: Some(setup.conversation_id),
        response_id: None,
        output_item_id: None,
    })
}

fn response_base(
    input: &ToolTurnInput<'_>,
    setup: &TurnSetup,
    events: &mut Vec<ToolTurnEvent>,
    uuids: &mut dyn FnMut() -> Uuid,
) -> (String, Option<String>) {
    input.response_id.map_or_else(
        || open_response(setup, events, uuids),
        |existing| {
            (
                str::to_owned(existing),
                input.output_item_id.map(str::to_owned),
            )
        },
    )
}

fn open_response(
    setup: &TurnSetup,
    events: &mut Vec<ToolTurnEvent>,
    uuids: &mut dyn FnMut() -> Uuid,
) -> (String, Option<String>) {
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
            conversation_id: setup.conversation_id.clone(),
            modalities: setup.modalities.clone(),
            temperature: setup.configuration.temperature.clone(),
            max_output_tokens: setup.configuration.max_output_tokens.clone(),
        },
    }));
    (response_id, item_base)
}

fn turn_usage(
    context: &mut ToolTurnContext<'_>,
    project_usage: &dyn Fn(&Usage) -> ResponseApiUsage,
) -> Result<Object, ToolTurnError> {
    let reported = consume_usage_metadata(context.state, context.frame);
    // the frame's grounding counters belong to no tool call, so this turn bills usage alone
    let chat = chat_usage(reported.as_ref(), None)?;
    let mut usage: Object = serde_json::from_slice(&serde_json::to_vec(&project_usage(&chat))?)?;
    add_usage_detail_aliases(&mut usage);
    Ok(usage)
}

fn done_event(
    setup: &TurnSetup,
    response_id: &str,
    calls: &[FunctionCallArgumentsDone],
    usage: Object,
    uuids: &mut dyn FnMut() -> Uuid,
) -> ToolTurnEvent {
    ToolTurnEvent::Done(ToolTurnDoneEvent {
        event_type: "response.done",
        event_id: format!("event_{}", uuids()),
        response: ToolTurnDoneResponse {
            object: "realtime.response",
            id: response_id.to_owned(),
            status: "completed",
            status_details: None,
            output: calls.iter().map(completed_item).collect(),
            conversation_id: setup.conversation_id.clone(),
            modalities: setup.modalities.clone(),
            usage,
            temperature: setup.configuration.temperature.clone(),
            max_output_tokens: setup.configuration.max_output_tokens.clone(),
        },
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
