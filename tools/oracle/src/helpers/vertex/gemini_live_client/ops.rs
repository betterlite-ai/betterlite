use std::mem;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::BundledCatalog;
use betterlite_prompts::arguments::json::Json;
// ast-grep-ignore: no-long-names
use betterlite_vertex::gemini_live_client::decode::decode_gemini_live_client_event;
use betterlite_vertex::gemini_live_client::sdk::{
    SdkDateTime, SdkError, SdkObject, SdkValue, encode_unserializable_types, to_object,
};
use betterlite_vertex::gemini_live_client::state::LiveSessionState;
use betterlite_vertex::gemini_live_client::transform::{
    FunctionResponseId, LiveClient, audio_mime_type,
};
use betterlite_vertex::gemini_live_setup::settings::LiveSetupSettings;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::warnings::Warnings;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const MODEL: &str = "gemini-2.5-flash";
// litellm.constants.DEFAULT_MAX_RECURSE_DEPTH, the depth the source encoder stops beyond
const MAX_DEPTH: i64 = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    op: Operation,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default)]
    cases: Vec<SdkCase>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Client,
    Sdk,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StepKind {
    Message,
    AudioPending,
    Mime,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    kind: StepKind,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    session_configuration_request: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCall {
    call_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SdkCase {
    shape: Shape,
    #[serde(default)]
    base64: Option<String>,
    #[serde(default)]
    datetime: Option<SdkDateTime>,
    #[serde(default)]
    depth: i64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Shape {
    Bytes,
    Datetime,
    BytesList,
    DatetimeList,
    EmptyList,
    ObjectBytes,
    ListObjectBytes,
    ListListBytes,
    MixedList,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    match input.op {
        Operation::Client => client(input),
        Operation::Sdk => sdk(input),
    }
}

fn client(input: &Input) -> Result<Value, Box<HelperError>> {
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
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let steps = with_default(subscriber, || {
        input
            .steps
            .iter()
            .map(|step| client_step(&mut live, step, &settings))
            .collect::<Result<Vec<_>, _>>()
    })?;
    Ok(json!({"steps": steps, "warnings": warnings.collected()}))
}

fn client_step(
    live: &mut LiveClient,
    step: &Step,
    settings: &LiveSetupSettings<'_>,
) -> Result<Value, Box<HelperError>> {
    match step.kind {
        StepKind::Message => message_step(live, step, settings),
        StepKind::AudioPending => {
            // the source's public reader answers only for the text-only transcribe model and zeroes the count
            let pending = mem::take(&mut live.state.unbilled_input_audio_bytes) > 0;
            Ok(json!({"pending": pending}))
        }
        StepKind::Mime => {
            let format = step
                .format
                .as_deref()
                .ok_or_else(|| failure("a mime step needs a format"))?;
            Ok(json!({"mime": audio_mime_type(format)}))
        }
    }
}

fn message_step(
    live: &mut LiveClient,
    step: &Step,
    settings: &LiveSetupSettings<'_>,
) -> Result<Value, Box<HelperError>> {
    for call in &step.tool_calls {
        live.state.tool_call_names.insert(&call.call_id, &call.name);
    }
    let text = step
        .message
        .as_deref()
        .ok_or_else(|| failure("a message step needs a message"))?;
    // ast-grep-ignore: no-long-names
    let event = decode_gemini_live_client_event(text.as_bytes(), usize::MAX).or_value_error()?;
    let messages = live
        .transform_realtime_request(
            &event,
            MODEL,
            step.session_configuration_request.as_deref(),
            settings,
        )
        .or_value_error()?;
    let parsed = messages
        .iter()
        .map(|message| serde_json::from_str::<Value>(message).or_value_error())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"messages": parsed}))
}

fn sdk(input: &Input) -> Result<Value, Box<HelperError>> {
    let cases = input
        .cases
        .iter()
        .map(sdk_case)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"cases": cases}))
}

fn sdk_case(case: &SdkCase) -> Result<Value, Box<HelperError>> {
    let value = match case.shape {
        Shape::Bytes => blob(case)?,
        Shape::Datetime => moment(case)?,
        Shape::BytesList => SdkValue::List(vec![blob(case)?, blob(case)?]),
        Shape::DatetimeList => SdkValue::List(vec![moment(case)?, moment(case)?]),
        Shape::EmptyList => SdkValue::List(Vec::new()),
        Shape::ObjectBytes => object(&[("inner", blob(case)?)]),
        Shape::ListObjectBytes => SdkValue::List(vec![object(&[("inner", blob(case)?)])]),
        Shape::ListListBytes => SdkValue::List(vec![SdkValue::List(vec![blob(case)?])]),
        Shape::MixedList => SdkValue::List(vec![blob(case)?, moment(case)?]),
    };
    let mut values = SdkObject::new();
    values.insert("value".to_owned(), value);
    let encoded = encode_unserializable_types(values, case.depth, MAX_DEPTH);
    match to_object(&encoded) {
        Ok(object) => serde_json::to_value(Json::Object(object)).or_value_error(),
        Err(SdkError::UnsupportedValue) => Ok(json!("raw")),
    }
}

fn blob(case: &SdkCase) -> Result<SdkValue, Box<HelperError>> {
    let text = case
        .base64
        .as_deref()
        .ok_or_else(|| failure("this shape needs a base64 value"))?;
    let bytes = STANDARD.decode(text).or_value_error()?;
    Ok(SdkValue::Bytes(bytes))
}

fn moment(case: &SdkCase) -> Result<SdkValue, Box<HelperError>> {
    let datetime = case
        .datetime
        .ok_or_else(|| failure("this shape needs a datetime value"))?;
    Ok(SdkValue::DateTime(datetime))
}

fn object(fields: &[(&str, SdkValue)]) -> SdkValue {
    SdkValue::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
