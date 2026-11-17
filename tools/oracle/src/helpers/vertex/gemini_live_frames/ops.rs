use betterlite_models::catalog::BundledCatalog;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::gemini_live_client::state::{LiveSessionState, ToolCallNames};
use betterlite_vertex::gemini_live_frames::decode::decode_gemini_live_frame_bytes;
use betterlite_vertex::gemini_live_frames::state::CallerState;
use betterlite_vertex::gemini_live_frames::transform::{
    ResponseContext, transform_realtime_response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

const DEFAULT_MODEL: &str = "gemini-2.5-flash";
const TRACE_ID: &str = "session-log";
// every identifier repeats the draw count in four 32-bit groups, the way the Python helper's counted uuid4 does
const UUID_PATTERN: u128 = 0x0000_0001_0000_0001_0000_0001_0000_0001;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    frames: Vec<String>,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default)]
    state: Option<CallerState>,
    #[serde(default)]
    pending_usage: Option<Object>,
    #[serde(default)]
    unbilled_input_audio_bytes: u64,
}

fn default_model() -> String {
    DEFAULT_MODEL.to_owned()
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let context = ResponseContext {
        model: &input.model,
        logging_session_id: TRACE_ID,
        catalog: catalog.as_ref(),
    };
    let mut session = LiveSessionState {
        tool_call_names: ToolCallNames::default(),
        pending_usage_metadata: input.pending_usage.clone(),
        unbilled_input_audio_bytes: input.unbilled_input_audio_bytes,
    };
    let mut caller = input.state.clone().unwrap_or_default();
    let mut drawn = 0_u128;
    let mut answers = Vec::with_capacity(input.frames.len());
    for text in &input.frames {
        let mut frame =
            decode_gemini_live_frame_bytes(text.as_bytes(), usize::MAX).or_value_error()?;
        let mut uuids = || {
            drawn = drawn.saturating_add(1);
            Uuid::from_u128(drawn.saturating_mul(UUID_PATTERN))
        };
        let answer =
            transform_realtime_response(&mut frame, &caller, &mut session, &context, &mut uuids)
                .or_value_error()?;
        caller = answer.state;
        answers.push(json!({
            "events": answer.events,
            "state": caller,
            "pending_usage": session.pending_usage_metadata,
        }));
    }
    Ok(json!({"frames": answers}))
}
