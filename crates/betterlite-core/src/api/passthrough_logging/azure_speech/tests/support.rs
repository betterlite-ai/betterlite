#![cfg(test)]

use std::io;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::super::{AzureSpeechPassthroughRequest, azure_speech_handler};

pub(super) const RESPONSE_TEXT: &str =
    r#"{"RecognitionStatus":"Success","DisplayText":"Hello world."}"#;

const WAV_FRAMES_PER_SECOND: u32 = 16_000;

pub(super) fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal fixture")
}

// The fixed response text every case forwards, and the marker kwarg the handler must carry through untouched.
pub(super) fn run(
    logging_obj: &StreamLoggingObject,
    lookup: &dyn PriceLookup,
    url_route: &str,
    request_body: Option<&[u8]>,
    response_body: &[u8],
) -> HandlerOutcome {
    azure_speech_handler(&AzureSpeechPassthroughRequest {
        logging_obj,
        url_route,
        response_body,
        request_body,
        result: RESPONSE_TEXT,
        kwargs: &PassThroughLoggingKwargs {
            entries: IndexMap::from([("user".to_owned(), Json::Text("u-1".to_owned()))]),
        },
        lookup,
    })
}

pub(super) fn standard_result() -> PassThroughResponse {
    PassThroughResponse::Standard(PassThroughResponseObject {
        response: Json::Text(RESPONSE_TEXT.to_owned()),
    })
}

pub(super) struct RecordingLookup {
    record: Option<PriceRecord>,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

impl RecordingLookup {
    pub(super) fn priced(body: &str) -> Self {
        Self {
            record: Some(serde_json::from_str::<PriceRecord>(body).expect("a price record")),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn unpriced() -> Self {
        Self {
            record: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PriceLookup for RecordingLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), custom_llm_provider.map(str::to_owned)));
        self.record.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn logging_object() -> StreamLoggingObject {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "pass-through".to_owned(),
            call_type: "pass_through_endpoint".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .expect("the fixture is RFC 3339"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    }
}

pub(super) fn details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

// The cost the caller's kwargs carry, read as the decimal the logger will bill.
pub(super) fn cost_of(outcome: &HandlerOutcome) -> Decimal {
    let value = outcome
        .kwargs
        .entries
        .get("response_cost")
        .expect("the returned kwargs carry a cost");
    number(value)
}

pub(super) fn recorded_cost(logging_obj: &StreamLoggingObject) -> Option<Decimal> {
    details(logging_obj).get("response_cost").map(number)
}

fn number(value: &Json) -> Decimal {
    let Json::Number(number) = value else {
        panic!("the response cost is a JSON number");
    };
    number
        .to_string()
        .parse()
        .expect("the response cost is a decimal")
}

pub(super) fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}

pub(super) fn logged_provider(logging_obj: &StreamLoggingObject) -> Option<String> {
    logging_obj
        .read(&mut |state| state.custom_llm_provider.clone())
        .flatten()
}

// A 16 kHz mono 16-bit PCM WAV, the container the native decoder measures.
pub(super) fn wav_seconds(seconds: u32) -> Vec<u8> {
    let frames = seconds
        .checked_mul(WAV_FRAMES_PER_SECOND)
        .expect("the fixture fits");
    let data = frames.checked_mul(2).expect("the fixture fits");
    let mut bytes = Vec::from(*b"RIFF");
    bytes.extend_from_slice(
        &data
            .checked_add(36)
            .expect("the fixture fits")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&WAV_FRAMES_PER_SECOND.to_le_bytes());
    bytes.extend_from_slice(&32_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    let zeroed = bytes
        .len()
        .checked_add(usize::try_from(data).expect("the data fits"))
        .expect("the fixture fits");
    bytes.resize(zeroed, 0_u8);
    bytes
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log is text")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs(level: tracing::Level) -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(captured.clone())
        .finish();
    (captured, set_default(subscriber))
}
