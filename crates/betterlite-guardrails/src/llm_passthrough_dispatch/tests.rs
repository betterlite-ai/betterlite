#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::{
    LlmPassthroughRouteHandler, PASSTHROUGH_TRANSLATION_KINDS, PassthroughTranslationKind,
};
use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_translation::{
    BaseTranslation, StreamTransformSink, StreamingScanKey, TranslationCall,
};

type Dispatch = LlmPassthroughRouteHandler<FakeProvider>;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn seen_marker() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([("seen_by".to_owned(), text("provider"))])
}

fn skip_line(provider: &str) -> String {
    format!("LlmPassthroughRouteHandler: no handler for provider={provider}, skipping guardrail")
}

struct FakeGuardrail {
    name: String,
}

impl GuardrailContract for FakeGuardrail {}

#[derive(Clone, Default)]
struct Ledger(Arc<Mutex<LedgerRows>>);

#[derive(Clone, Default, Debug, PartialEq)]
struct LedgerRows {
    inputs: Vec<SeenInput>,
    outputs: Vec<SeenOutput>,
    streams: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct SeenInput {
    guardrail: String,
    data: BTreeMap<String, GuardrailValue>,
    log_target: Option<GuardrailLogTarget>,
}

#[derive(Clone, Debug, PartialEq)]
struct SeenOutput {
    response: GuardrailValue,
    guardrail: String,
    request_data: Option<BTreeMap<String, GuardrailValue>>,
    user_api_key_dict: Option<BTreeMap<String, GuardrailValue>>,
    log_target: Option<GuardrailLogTarget>,
    sink: Option<IndexMap<i64, String>>,
    deliver_ended_stream_rewrites: bool,
}

fn rows(ledger: &Ledger) -> LedgerRows {
    ledger.0.lock().expect("ledger").clone()
}

struct FakeProvider {
    ledger: Ledger,
    fail: bool,
    response: GuardrailValue,
}

fn provider() -> FakeProvider {
    FakeProvider {
        ledger: Ledger::default(),
        fail: false,
        response: text("rewritten"),
    }
}

fn failing_provider() -> FakeProvider {
    FakeProvider {
        ledger: Ledger::default(),
        fail: true,
        response: text("unused"),
    }
}

impl FakeProvider {
    fn record_output(
        &self,
        response: &GuardrailValue,
        guardrail: &FakeGuardrail,
        call: &TranslationCall<'_, GuardrailLogTarget>,
    ) {
        let record = SeenOutput {
            response: response.clone(),
            guardrail: guardrail.name.clone(),
            request_data: call.request_data.as_deref().cloned(),
            user_api_key_dict: call.user_api_key_dict.cloned(),
            log_target: call.logging_obj.as_deref().cloned(),
            sink: call
                .stream_transform_sink
                .as_deref()
                .map(|sink| sink.mutated_text_per_choice.clone()),
            deliver_ended_stream_rewrites: call.deliver_ended_stream_rewrites,
        };
        self.ledger.0.lock().expect("ledger").outputs.push(record);
    }
}

fn apply_output_mutations(call: TranslationCall<'_, GuardrailLogTarget>) {
    if let Some(target) = call.logging_obj {
        target.model_call_params = Some(seen_marker());
    }
    if let Some(fields) = call.request_data {
        fields.insert("seen_by_provider".to_owned(), GuardrailValue::Boolean(true));
    }
    if let Some(sink) = call.stream_transform_sink {
        sink.mutated_text_per_choice.insert(9, "seen".to_owned());
    }
}

impl BaseTranslation for FakeProvider {
    type Guardrail = FakeGuardrail;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        let log_target = logging_obj.as_deref().cloned();
        let record = SeenInput {
            guardrail: guardrail.name.clone(),
            data: data.clone(),
            log_target,
        };
        self.ledger.0.lock().expect("ledger").inputs.push(record);
        if let Some(target) = logging_obj {
            target.model_call_params = Some(seen_marker());
        }
        data.insert("seen_by_provider".to_owned(), GuardrailValue::Boolean(true));
        if self.fail {
            return ready(Err(GuardrailFailure::Other("input failed".to_owned())));
        }
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        self.record_output(&response, guardrail, &call);
        apply_output_mutations(call);
        if self.fail {
            return ready(Err(GuardrailFailure::Other("output failed".to_owned())));
        }
        ready(Ok(self.response.clone()))
    }

    fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        responses_so_far.push(text("provider chunk"));
        self.ledger
            .0
            .lock()
            .expect("ledger")
            .streams
            .push("process_streaming_response".to_owned());
        ready(Ok(()))
    }

    fn get_streaming_scan_key(
        &self,
        _responses_so_far: &[GuardrailValue],
    ) -> Option<StreamingScanKey> {
        Some(StreamingScanKey {
            texts: vec!["provider".to_owned()],
            tool_calls: Vec::new(),
            stream_ended: false,
            tool_calls_in_flight: false,
        })
    }

    fn build_block_sse_chunks(
        &self,
        _failure: &Self::Failure,
        _stream_started: bool,
        _responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<Vec<u8>>> {
        Some(vec![b"block".to_vec()])
    }

    fn build_stream_error_items(
        &self,
        _failure: &Self::Failure,
        _responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<GuardrailValue>> {
        Some(vec![text("provider error")])
    }

    fn get_structured_messages(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
    ) -> Option<Vec<GuardrailMessage>> {
        Some(Vec::new())
    }

    fn extract_request_tool_names(&self, _data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        vec!["provider-tool".to_owned()]
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
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

fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let subscriber = fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(captured.clone())
        .finish();
    (captured, set_default(subscriber))
}

fn logged_text(captured: &Captured) -> String {
    let bytes = captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    String::from_utf8(bytes).expect("captured log is UTF-8")
}

fn routed_call<'call>(
    user_key: &'call BTreeMap<String, GuardrailValue>,
    request_data: &'call mut BTreeMap<String, GuardrailValue>,
    log_target: &'call mut GuardrailLogTarget,
    sink: &'call mut StreamTransformSink,
) -> TranslationCall<'call, GuardrailLogTarget> {
    TranslationCall {
        user_api_key_dict: Some(user_key),
        request_data: Some(request_data),
        logging_obj: Some(log_target),
        stream_transform_sink: Some(sink),
        deliver_ended_stream_rewrites: true,
    }
}

#[tokio::test]
async fn input_routes_to_the_named_provider() {
    let bedrock = provider();
    let vertex = provider();
    let bedrock_ledger = bedrock.ledger.clone();
    let vertex_ledger = vertex.ledger.clone();
    let handler = LlmPassthroughRouteHandler::new(vec![
        ("bedrock".to_owned(), bedrock),
        ("vertex".to_owned(), vertex),
    ]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let mut data = BTreeMap::from([
        ("custom_llm_provider".to_owned(), text("vertex")),
        ("model".to_owned(), text("glm-4")),
    ]);
    let delivered = data.clone();
    let mut log_target = GuardrailLogTarget::default();
    handler
        .process_input_messages(&mut data, &guardrail, Some(&mut log_target))
        .await
        .unwrap();
    assert!(rows(&bedrock_ledger).inputs.is_empty());
    let entries = rows(&vertex_ledger).inputs;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].guardrail, "pii");
    assert_eq!(entries[0].data, delivered);
    assert_eq!(entries[0].log_target, Some(GuardrailLogTarget::default()));
    assert_eq!(
        data.get("seen_by_provider"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(log_target.model_call_params, Some(seen_marker()));
}

#[rstest]
#[case::missing(None, "None")]
#[case::null(Some(GuardrailValue::Null), "None")]
#[case::empty(Some(GuardrailValue::Text(String::new())), "")]
#[case::cased(Some(GuardrailValue::Text("Bedrock".to_owned())), "Bedrock")]
#[case::padded(Some(GuardrailValue::Text("bedrock ".to_owned())), "bedrock ")]
#[case::unknown(Some(GuardrailValue::Text("vertex_ai".to_owned())), "vertex_ai")]
#[case::integer(Some(GuardrailValue::SignedInteger(7)), "7")]
#[case::boolean(Some(GuardrailValue::Boolean(true)), "true")]
#[case::array(
    Some(GuardrailValue::Array(vec![text("bedrock")])),
    "[\"bedrock\"]"
)]
#[case::object(
    Some(GuardrailValue::Object(BTreeMap::from([(
        "name".to_owned(),
        text("bedrock")
    )]))),
    "{\"name\":\"bedrock\"}"
)]
#[tokio::test]
async fn input_skips_without_a_usable_provider(
    #[case] provider_value: Option<GuardrailValue>,
    #[case] expected: &str,
) {
    let bedrock = provider();
    let ledger = bedrock.ledger.clone();
    let handler = LlmPassthroughRouteHandler::new(vec![("bedrock".to_owned(), bedrock)]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let mut data = BTreeMap::from([("model".to_owned(), text("glm-4"))]);
    if let Some(value) = provider_value {
        data.insert("custom_llm_provider".to_owned(), value);
    }
    let untouched = data.clone();
    let (captured, _guard) = capture_logs();
    handler
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(rows(&ledger).inputs.is_empty());
    assert_eq!(data, untouched);
    let logs = logged_text(&captured);
    assert!(logs.contains(&skip_line(expected)), "{logs}");
}

#[tokio::test]
async fn input_skips_an_empty_name_even_when_the_table_has_one() {
    let empty = provider();
    let ledger = empty.ledger.clone();
    let handler = LlmPassthroughRouteHandler::new(vec![(String::new(), empty)]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let mut data = BTreeMap::from([("custom_llm_provider".to_owned(), text(""))]);
    handler
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(rows(&ledger).inputs.is_empty());
}

#[tokio::test]
async fn input_provider_failure_passes_through() {
    let handler = LlmPassthroughRouteHandler::new(vec![("bedrock".to_owned(), failing_provider())]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let mut data = BTreeMap::from([("custom_llm_provider".to_owned(), text("bedrock"))]);
    let result = handler
        .process_input_messages(&mut data, &guardrail, None)
        .await;
    assert_eq!(
        result,
        Err(GuardrailFailure::Other("input failed".to_owned()))
    );
}

#[tokio::test]
async fn input_a_later_duplicate_provider_wins() {
    let first = provider();
    let second = provider();
    let first_ledger = first.ledger.clone();
    let second_ledger = second.ledger.clone();
    let handler = LlmPassthroughRouteHandler::new(vec![
        ("bedrock".to_owned(), first),
        ("bedrock".to_owned(), second),
    ]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let mut data = BTreeMap::from([("custom_llm_provider".to_owned(), text("bedrock"))]);
    handler
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(rows(&first_ledger).inputs.is_empty());
    assert_eq!(rows(&second_ledger).inputs.len(), 1);
}

#[tokio::test]
async fn output_routes_by_the_request_data_provider() {
    let bedrock = provider();
    let bedrock_ledger = bedrock.ledger.clone();
    let handler = LlmPassthroughRouteHandler::new(vec![("bedrock".to_owned(), bedrock)]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let user_key = BTreeMap::from([("user_id".to_owned(), text("u-1"))]);
    let mut log_target = GuardrailLogTarget::default();
    let mut sink = StreamTransformSink::default();
    sink.mutated_text_per_choice.insert(1, "abc".to_owned());
    let mut request_data = BTreeMap::from([("custom_llm_provider".to_owned(), text("bedrock"))]);
    let delivered = request_data.clone();
    let delivered_sink = sink.mutated_text_per_choice.clone();
    let call = routed_call(&user_key, &mut request_data, &mut log_target, &mut sink);
    let response = handler
        .process_output_response(text("original"), &guardrail, call)
        .await
        .unwrap();
    assert_eq!(response, text("rewritten"));
    let entries = rows(&bedrock_ledger).outputs;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].response, text("original"));
    assert_eq!(entries[0].guardrail, "pii");
    assert_eq!(entries[0].request_data, Some(delivered));
    assert_eq!(
        entries[0].user_api_key_dict,
        Some(BTreeMap::from([("user_id".to_owned(), text("u-1"))]))
    );
    assert_eq!(entries[0].log_target, Some(GuardrailLogTarget::default()));
    assert_eq!(entries[0].sink, Some(delivered_sink));
    assert!(entries[0].deliver_ended_stream_rewrites);
}

#[tokio::test]
async fn output_keeps_the_provider_edits_visible_to_the_caller() {
    let bedrock = provider();
    let handler = LlmPassthroughRouteHandler::new(vec![("bedrock".to_owned(), bedrock)]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let user_key = BTreeMap::from([("user_id".to_owned(), text("u-1"))]);
    let mut log_target = GuardrailLogTarget::default();
    let mut sink = StreamTransformSink::default();
    let mut request_data = BTreeMap::from([("custom_llm_provider".to_owned(), text("bedrock"))]);
    let call = routed_call(&user_key, &mut request_data, &mut log_target, &mut sink);
    handler
        .process_output_response(text("original"), &guardrail, call)
        .await
        .unwrap();
    assert_eq!(
        request_data.get("seen_by_provider"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(
        sink.mutated_text_per_choice.get(&9),
        Some(&"seen".to_owned())
    );
    assert_eq!(log_target.model_call_params, Some(seen_marker()));
}

#[rstest]
#[case::missing_request_data(None, "None")]
#[case::no_provider_key(Some(BTreeMap::new()), "None")]
#[case::null_provider(
    Some(BTreeMap::from([(
        "custom_llm_provider".to_owned(),
        GuardrailValue::Null
    )])),
    "None"
)]
#[case::empty_provider(
    Some(BTreeMap::from([(
        "custom_llm_provider".to_owned(),
        GuardrailValue::Text(String::new())
    )])),
    ""
)]
#[case::non_text_provider(
    Some(BTreeMap::from([(
        "custom_llm_provider".to_owned(),
        GuardrailValue::SignedInteger(3)
    )])),
    "3"
)]
#[case::unknown_provider(
    Some(BTreeMap::from([(
        "custom_llm_provider".to_owned(),
        text("vertex_ai")
    )])),
    "vertex_ai"
)]
#[case::cased_provider(
    Some(BTreeMap::from([(
        "custom_llm_provider".to_owned(),
        text("Bedrock")
    )])),
    "Bedrock"
)]
#[tokio::test]
async fn output_skips_without_a_usable_provider(
    #[case] request_data: Option<BTreeMap<String, GuardrailValue>>,
    #[case] expected: &str,
) {
    let mut request_map = request_data;
    let bedrock = provider();
    let ledger = bedrock.ledger.clone();
    let handler = LlmPassthroughRouteHandler::new(vec![("bedrock".to_owned(), bedrock)]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let response = GuardrailValue::Object(BTreeMap::from([
        ("custom_llm_provider".to_owned(), text("bedrock")),
        ("model".to_owned(), text("glm-4")),
    ]));
    let call = TranslationCall {
        user_api_key_dict: None,
        request_data: request_map.as_mut(),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    let (captured, _guard) = capture_logs();
    let returned = handler
        .process_output_response(response.clone(), &guardrail, call)
        .await
        .unwrap();
    assert!(rows(&ledger).outputs.is_empty());
    assert_eq!(returned, response);
    let logs = logged_text(&captured);
    assert!(logs.contains(&skip_line(expected)), "{logs}");
}

#[tokio::test]
async fn output_provider_failure_passes_through() {
    let handler = LlmPassthroughRouteHandler::new(vec![("bedrock".to_owned(), failing_provider())]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let mut request_data = BTreeMap::from([("custom_llm_provider".to_owned(), text("bedrock"))]);
    let call = TranslationCall {
        user_api_key_dict: None,
        request_data: Some(&mut request_data),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    let result = handler
        .process_output_response(text("original"), &guardrail, call)
        .await;
    assert_eq!(
        result,
        Err(GuardrailFailure::Other("output failed".to_owned()))
    );
}

#[tokio::test]
async fn defaults_streaming_keeps_the_items_and_calls_no_provider() {
    let bedrock = provider();
    let ledger = bedrock.ledger.clone();
    let handler = LlmPassthroughRouteHandler::new(vec![("bedrock".to_owned(), bedrock)]);
    let guardrail = FakeGuardrail {
        name: "pii".to_owned(),
    };
    let mut request_data = BTreeMap::from([("custom_llm_provider".to_owned(), text("bedrock"))]);
    let mut responses = vec![text("chunk")];
    let original = responses.clone();
    let call = TranslationCall {
        user_api_key_dict: None,
        request_data: Some(&mut request_data),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    handler
        .process_streaming_response(&mut responses, &guardrail, call)
        .await
        .unwrap();
    assert_eq!(responses, original);
    assert!(rows(&ledger).streams.is_empty());
}

#[test]
fn defaults_scan_key_and_stream_builders_are_absent() {
    let handler = Dispatch::new(vec![("bedrock".to_owned(), provider())]);
    let failure = GuardrailFailure::Other("blocked".to_owned());
    let chunks: &[GuardrailValue] = &[text("chunk")];
    assert!(handler.get_streaming_scan_key(chunks).is_none());
    assert!(
        handler
            .build_block_sse_chunks(&failure, true, Some(chunks))
            .is_none()
    );
    assert!(
        handler
            .build_stream_error_items(&failure, Some(chunks))
            .is_none()
    );
}

#[test]
fn defaults_structured_messages_and_tool_names_are_empty() {
    let handler = Dispatch::new(vec![("bedrock".to_owned(), provider())]);
    let data = BTreeMap::from([("model".to_owned(), text("glm-4"))]);
    assert!(handler.get_structured_messages(&data).is_none());
    assert!(handler.extract_request_tool_names(&data).is_empty());
}

#[test]
fn defaults_class_flags_are_false() {
    let delivers_rewrites = <Dispatch as BaseTranslation>::DELIVERS_ENDED_STREAM_REWRITES;
    let assembles_response = <Dispatch as BaseTranslation>::ASSEMBLES_STREAMED_RESPONSE;
    assert!(!delivers_rewrites);
    assert!(!assembles_response);
}

#[test]
fn mapping_table_maps_the_two_passthrough_call_types() {
    assert_eq!(
        PASSTHROUGH_TRANSLATION_KINDS,
        [
            (
                CallType::PassThroughEndpoint,
                PassthroughTranslationKind::Endpoint
            ),
            (
                CallType::AllmPassthroughRoute,
                PassthroughTranslationKind::LlmRoute
            ),
        ]
    );
}

#[test]
fn mapping_table_has_no_row_for_the_sync_llm_route() {
    assert_eq!(PASSTHROUGH_TRANSLATION_KINDS.len(), 2);
    assert!(
        !PASSTHROUGH_TRANSLATION_KINDS
            .iter()
            .any(|(call_type, _kind)| *call_type == CallType::LlmPassthroughRoute)
    );
}
