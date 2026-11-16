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

struct Context<'context> {
    live: &'context mut LiveClient,
    settings: &'context LiveSetupSettings<'context>,
    uuids: &'context mut dyn FnMut() -> Uuid,
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
    let mut context = Context {
        live: &mut live,
        settings: &settings,
        uuids: &mut uuids,
    };
    let values = steps
        .iter()
        .map(|step| step_value(&mut context, step))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"steps": values}))
}

fn step_value(context: &mut Context<'_>, step: &Step) -> Result<Value, Box<HelperError>> {
    match step.kind {
        StepKind::Calls => calls_value(context, step),
        StepKind::Message => Ok(json!({
            "messages": client_messages(context, message_text(step)?)?
        })),
        StepKind::Audio => audio_value(context, audio_bytes(step)?),
        StepKind::StandaloneUsage => Ok(usage_value(context, standalone_usage(step)?)),
        StepKind::ResponseDone => {
            let mut frame = done_frame(step)?;
            let input = ResponseDoneInput {
                response_id: step.response_id.as_deref(),
                conversation_id: step.conversation_id.as_deref(),
                output_items: &step.output_items,
                session_configuration_request: step.session_configuration_request.as_deref(),
            };
            let event = transform_response_done_event(
                &mut context.live.state,
                &mut frame,
                &input,
                &|usage| usage_details(Some(usage)),
                context.uuids,
            )
            .or_value_error()?;
            Ok(json!({"event": event}))
        }
        StepKind::Close => close_value(context, close_model(step)?),
    }
}

fn message_text(step: &Step) -> Result<&str, Box<HelperError>> {
    step.message
        .as_deref()
        .ok_or("a message step needs a message")
        .or_value_error()
}

fn audio_bytes(step: &Step) -> Result<usize, Box<HelperError>> {
    step.audio_bytes
        .ok_or("an audio step needs audio_bytes")
        .or_value_error()
}

fn standalone_usage(step: &Step) -> Result<&Object, Box<HelperError>> {
    step.usage
        .as_ref()
        .ok_or("a standalone_usage step needs a usage")
        .or_value_error()
}

fn done_frame(step: &Step) -> Result<ServerMessage, Box<HelperError>> {
    step.frame
        .clone()
        .ok_or("a response_done step needs a frame")
        .or_value_error()
}

fn close_model(step: &Step) -> Result<&str, Box<HelperError>> {
    step.model
        .as_deref()
        .ok_or("a close step needs a model")
        .or_value_error()
}

fn calls_value(context: &mut Context<'_>, step: &Step) -> Result<Value, Box<HelperError>> {
    let message = tool_call_message(step)?;
    let names = &mut context.live.state.tool_call_names;
    let events = transform_tool_call_events(names, &message, call_ids(step), context.uuids);
    Ok(json!({"events": events}))
}

fn call_ids(step: &Step) -> ToolCallIds<'_> {
    ToolCallIds {
        response_id: step.response_id.as_deref(),
        output_item_id: step.output_item_id.as_deref(),
    }
}

// the audio frame is B1379's output, so only the count of frames sent is compared
fn audio_value(context: &mut Context<'_>, bytes: usize) -> Result<Value, Box<HelperError>> {
    let audio = STANDARD.encode(vec![0_u8; bytes]);
    let append = json!({"type": "input_audio_buffer.append", "audio": audio});
    let messages = client_messages(context, &append.to_string())?;
    Ok(json!({"sent": messages.len()}))
}

// the source buffers a dict usageMetadata from a frame that emits no event
fn usage_value(context: &mut Context<'_>, usage: &Object) -> Value {
    context.live.state.pending_usage_metadata = Some(usage.clone());
    json!({"events": []})
}

fn close_value(context: &mut Context<'_>, model: &str) -> Result<Value, Box<HelperError>> {
    let state = &mut context.live.state;
    let catalog = context.settings.catalog;
    let usage = consume_transcript_estimate(state, catalog, model).or_value_error()?;
    Ok(json!({"usage": usage}))
}

fn tool_call_message(step: &Step) -> Result<ToolCallMessage, Box<HelperError>> {
    if let Some(message) = &step.tool_call {
        return Ok(message.clone());
    }
    let generated = step
        .burst
        .as_ref()
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

fn client_messages(context: &mut Context<'_>, text: &str) -> Result<Vec<Value>, Box<HelperError>> {
    // ast-grep-ignore: no-long-names
    let event = decode_gemini_live_client_event(text.as_bytes(), usize::MAX).or_value_error()?;
    let messages = context
        .live
        .transform_realtime_request(&event, MODEL, None, context.settings)
        .or_value_error()?;
    let mut parsed = Vec::with_capacity(messages.len());
    for message in &messages {
        parsed.push(serde_json::from_str::<Value>(message).or_value_error()?);
    }
    Ok(parsed)
}
