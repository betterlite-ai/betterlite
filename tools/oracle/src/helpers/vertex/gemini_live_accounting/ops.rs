use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_core::api::responses_output_details::usage_details;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::gemini_live_accounting::aliases::add_usage_detail_aliases;
use betterlite_vertex::gemini_live_accounting::response_done::{
    ResponseDoneInput, transform_response_done_event,
};
use betterlite_vertex::gemini_live_accounting::tool_calls::{
    FunctionCall, ToolCallIds, ToolCallMessage, transform_tool_call_events,
};
use betterlite_vertex::gemini_live_accounting::transcription::consume_transcript_estimate;
// ast-grep-ignore: no-long-names
use betterlite_vertex::gemini_live_client::decode::decode_gemini_live_client_event;
use betterlite_vertex::gemini_live_client::records::ServerMessage;
use betterlite_vertex::gemini_live_client::state::LiveSessionState;
use betterlite_vertex::gemini_live_client::transform::{FunctionResponseId, LiveClient};
use betterlite_vertex::gemini_live_setup::settings::LiveSetupSettings;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

const MODEL: &str = "gemini-2.5-flash";
// every identifier repeats the draw count in four 32-bit groups, the way the Python helper's counted uuid4 does
const UUID_PATTERN: u128 = 0x0000_0001_0000_0001_0000_0001_0000_0001;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    op: Operation,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default)]
    usage: Option<Object>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Steps,
    Aliases,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StepKind {
    Calls,
    Message,
    Audio,
    StandaloneUsage,
    ResponseDone,
    Close,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    kind: StepKind,
    #[serde(default)]
    tool_call: Option<ToolCallMessage>,
    #[serde(default)]
    burst: Option<Burst>,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    output_item_id: Option<String>,
    #[serde(default)]
    output_items: Vec<Object>,
    #[serde(default)]
    session_configuration_request: Option<String>,
    #[serde(default)]
    frame: Option<ServerMessage>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    audio_bytes: Option<usize>,
    #[serde(default)]
    usage: Option<Object>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Burst {
    count: usize,
    id_prefix: String,
    name_prefix: String,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    match input.op {
        Operation::Aliases => {
            let mut usage = input
                .usage
                .clone()
                .ok_or("an aliases call needs a usage")
                .or_value_error()?;
            add_usage_detail_aliases(&mut usage);
            Ok(json!({"usage": usage}))
        }
        Operation::Steps => run_steps(&input.steps),
    }
}

fn run_steps(steps: &[Step]) -> Result<Value, Box<HelperError>> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let schema = SchemaSettings::default();
    let settings = LiveSetupSettings {
        catalog: catalog.as_ref(),
        schema: &schema,
        defer_setup: false,
    };
    let mut live = LiveClient {
        state: LiveSessionState::default(),
        function_response_id: FunctionResponseId::Include,
    };
    let mut drawn = 0_u128;
    let mut uuids = || {
        drawn = drawn.saturating_add(1);
        Uuid::from_u128(drawn.saturating_mul(UUID_PATTERN))
    };
    let values = steps
        .iter()
        .map(|step| step_value(step, &mut live, &settings, &mut uuids))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"steps": values}))
}

fn step_value(
    step: &Step,
    live: &mut LiveClient,
    settings: &LiveSetupSettings<'_>,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<Value, Box<HelperError>> {
    match step.kind {
        StepKind::Calls => calls_value(step, live, uuids),
        StepKind::Message => message_value(step, live, settings),
        StepKind::Audio => audio_value(step, live, settings),
        StepKind::StandaloneUsage => usage_value(step, live),
        StepKind::ResponseDone => done_value(step, live, uuids),
        StepKind::Close => close_value(step, live, settings),
    }
}

fn calls_value(
    step: &Step,
    live: &mut LiveClient,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<Value, Box<HelperError>> {
    let message = tool_call_message(step.tool_call.as_ref(), step.burst.as_ref())?;
    let ids = ToolCallIds {
        response_id: step.response_id.as_deref(),
        output_item_id: step.output_item_id.as_deref(),
    };
    let events = transform_tool_call_events(&mut live.state.tool_call_names, &message, ids, uuids);
    Ok(json!({"events": events}))
}

fn message_value(
    step: &Step,
    live: &mut LiveClient,
    settings: &LiveSetupSettings<'_>,
) -> Result<Value, Box<HelperError>> {
    let message = step
        .message
        .as_deref()
        .ok_or("a message step needs a message")
        .or_value_error()?;
    let messages = client_messages(live, message, settings)?;
    Ok(json!({"messages": messages}))
}

fn audio_value(
    step: &Step,
    live: &mut LiveClient,
    settings: &LiveSetupSettings<'_>,
) -> Result<Value, Box<HelperError>> {
    let audio_bytes = step
        .audio_bytes
        .ok_or("an audio step needs audio_bytes")
        .or_value_error()?;
    let audio = STANDARD.encode(vec![0_u8; audio_bytes]);
    let append = json!({"type": "input_audio_buffer.append", "audio": audio});
    // the audio frame is B1379's output, so only the count of frames sent is compared
    let messages = client_messages(live, &append.to_string(), settings)?;
    Ok(json!({"sent": messages.len()}))
}

fn usage_value(step: &Step, live: &mut LiveClient) -> Result<Value, Box<HelperError>> {
    let usage = step
        .usage
        .as_ref()
        .ok_or("a standalone_usage step needs a usage")
        .or_value_error()?;
    // the source buffers a dict usageMetadata from a frame that emits no event
    live.state.pending_usage_metadata = Some(usage.clone());
    Ok(json!({"events": []}))
}

fn done_value(
    step: &Step,
    live: &mut LiveClient,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<Value, Box<HelperError>> {
    let mut frame = step
        .frame
        .clone()
        .ok_or("a response_done step needs a frame")
        .or_value_error()?;
    let input = ResponseDoneInput {
        response_id: step.response_id.as_deref(),
        conversation_id: step.conversation_id.as_deref(),
        output_items: &step.output_items,
        session_configuration_request: step.session_configuration_request.as_deref(),
    };
    let event = transform_response_done_event(
        &mut live.state,
        &mut frame,
        &input,
        &|usage| usage_details(Some(usage)),
        uuids,
    )
    .or_value_error()?;
    Ok(json!({"event": event}))
}

fn close_value(
    step: &Step,
    live: &mut LiveClient,
    settings: &LiveSetupSettings<'_>,
) -> Result<Value, Box<HelperError>> {
    let model = step
        .model
        .as_deref()
        .ok_or("a close step needs a model")
        .or_value_error()?;
    let usage =
        consume_transcript_estimate(&mut live.state, settings.catalog, model).or_value_error()?;
    Ok(json!({"usage": usage}))
}

fn tool_call_message(
    tool_call: Option<&ToolCallMessage>,
    burst: Option<&Burst>,
) -> Result<ToolCallMessage, Box<HelperError>> {
    if let Some(message) = tool_call {
        return Ok(message.clone());
    }
    let generated = burst
        .ok_or("a calls step needs a tool_call or a burst")
        .or_value_error()?;
    let function_calls = (0..generated.count)
        .map(|index| FunctionCall {
            id: Some(format!("{}{index}", generated.id_prefix)),
            name: format!("{}{index}", generated.name_prefix),
            args: Json::Object(IndexMap::new()),
        })
        .collect();
    Ok(ToolCallMessage { function_calls })
}

fn client_messages(
    live: &mut LiveClient,
    text: &str,
    settings: &LiveSetupSettings<'_>,
) -> Result<Vec<Value>, Box<HelperError>> {
    // ast-grep-ignore: no-long-names
    let event = decode_gemini_live_client_event(text.as_bytes(), usize::MAX).or_value_error()?;
    let messages = live
        .transform_realtime_request(&event, MODEL, None, settings)
        .or_value_error()?;
    messages
        .iter()
        .map(|message| serde_json::from_str::<Value>(message).or_value_error())
        .collect()
}
