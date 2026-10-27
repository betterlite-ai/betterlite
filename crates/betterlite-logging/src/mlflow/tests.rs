#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future as _;
use std::pin::pin;
use std::sync::{Arc, Barrier, Mutex};
use std::task::{Context, Poll, Waker};
use std::{io, iter, thread};

use betterlite_logging::mlflow::attributes::{
    Details, Kwargs, ProviderParams, Standard, transform_tags,
};
use betterlite_logging::mlflow::client::{Client, ClientError, EndRequest, StartRequest};
use betterlite_logging::mlflow::logger::{Call, Logger};
use betterlite_logging::mlflow::model::{
    Choice, Event, ExceptionInfo, OptionalParams, Payload, Response, Span, SpanKind, Status,
    Timestamp,
};
use chrono::{TimeZone as _, Utc};
use rstest::rstest;
use serde::Serialize;
use tracing::subscriber;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct Text(String);

impl Payload for Text {
    fn is_truthy(&self) -> bool {
        !self.0.is_empty()
    }
}

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Active,
    Start,
    Event,
    Messages,
    Tools,
    End,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Record {
    Start(Box<StartRequest<Text>>, Option<Span>),
    Event(Event),
    Messages(Vec<Text>),
    Tools(Vec<Text>),
    End {
        child: bool,
        response: Response<Text>,
        status: Status,
        time: i64,
    },
}

#[derive(Default)]
struct State {
    records: Vec<Record>,
    fail: Option<Stage>,
}

struct Mock {
    state: Arc<Mutex<State>>,
    parent: Option<Span>,
    chat: bool,
}

impl Mock {
    fn record(&self, phase: Stage, record: Record) -> Result<(), ClientError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_poisoned| ClientError::Operation("lock".to_owned()))?;
        state.records.push(record);
        let fail = state.fail;
        drop(state);
        if fail == Some(phase) {
            Err(ClientError::Operation("injected".to_owned()))
        } else {
            Ok(())
        }
    }
    fn begin(
        &self,
        request: StartRequest<Text>,
        parent: Option<Span>,
    ) -> Result<Span, ClientError> {
        self.record(
            Stage::Start,
            Record::Start(Box::new(request), parent.clone()),
        )?;
        Ok(parent.map_or_else(
            || Span {
                request_id: "trace".to_owned(),
                span_id: "root".to_owned(),
                parent_id: None,
            },
            |active| Span {
                request_id: active.request_id,
                span_id: "child".to_owned(),
                parent_id: Some(active.span_id),
            },
        ))
    }
    fn finish(&self, request: &EndRequest<'_, Text>, child: bool) -> Result<(), ClientError> {
        let record = Record::End {
            child,
            response: request.outputs.clone(),
            status: request.status,
            time: request.end_time_ns,
        };
        self.record(Stage::End, record)
    }
}

impl Client for Mock {
    type Data = Text;

    fn active_span(&self) -> Result<Option<Span>, ClientError> {
        if self
            .state
            .lock()
            .map_err(|_poisoned| ClientError::Operation("lock".to_owned()))?
            .fail
            == Some(Stage::Active)
        {
            Err(ClientError::Operation("active".to_owned()))
        } else {
            Ok(self.parent.clone())
        }
    }
    fn start_trace(&self, request: StartRequest<Text>) -> Result<Span, ClientError> {
        self.begin(request, None)
    }
    fn start_span(&self, request: StartRequest<Text>, parent: &Span) -> Result<Span, ClientError> {
        self.begin(request, Some(parent.clone()))
    }
    fn add_event(&self, _span: &Span, event: Event) -> Result<(), ClientError> {
        self.record(Stage::Event, Record::Event(event))
    }
    fn chat_available(&self) -> bool {
        self.chat
    }
    fn set_chat_messages(&self, _span: &Span, messages: Vec<Text>) -> Result<(), ClientError> {
        self.record(Stage::Messages, Record::Messages(messages))
    }
    fn set_chat_tools(&self, _span: &Span, tools: Vec<Text>) -> Result<(), ClientError> {
        self.record(Stage::Tools, Record::Tools(tools))
    }
    fn end_trace(&self, _span: &Span, request: EndRequest<'_, Text>) -> Result<(), ClientError> {
        self.finish(&request, false)
    }
    fn end_span(&self, _span: &Span, request: EndRequest<'_, Text>) -> Result<(), ClientError> {
        self.finish(&request, true)
    }
}

fn fixture(parent: Option<Span>, chat: bool, capacity: usize) -> (Logger<Mock>, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let client = Mock {
        state: Arc::clone(&state),
        parent,
        chat,
    };
    (Logger::new(client, capacity).unwrap(), state)
}

fn times() -> (Timestamp, Timestamp) {
    let start = Utc.timestamp_opt(1, 123).single().unwrap();
    (start, Utc.timestamp_opt(2, 456).single().unwrap())
}

fn response() -> Response<Text> {
    let choices = vec![Choice {
        message: Some(Text::from("hello")),
        delta_method: None,
    }];
    Response {
        choices,
        data: None,
    }
}

fn chunk() -> Response<Text> {
    let mut output = response();
    let choice = output.choices.first_mut().unwrap();
    choice.message = None;
    choice.delta_method =
        Some("<bound method BaseModel.model_dump of Delta(content='\u{e9}\u{1f600}')>".to_owned());
    output
}

fn stream_kwargs(final_chunk: bool) -> Kwargs<Text> {
    let mut kwargs = request(&[]);
    kwargs.stream = true;
    kwargs.litellm_call_id = Some("call-1".to_owned());
    if final_chunk {
        kwargs.complete_streaming_response = Some(response());
    }
    kwargs
}

#[test]
fn non_streaming_success_ends_trace_and_extracts_chat_data() {
    let (logger, state) = fixture(None, true, 8);
    let mut kwargs = request(&[]);
    kwargs.messages = Some(vec![Text::from("hi")]);
    kwargs.tools = Some(vec![Text::from("search")]);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &response(),
        start,
        end,
    });
    let records = &state.lock().unwrap().records;
    assert!(
        matches!(&records[0], Record::Start(request, None) if request.name == "litellm-completion" && request.start_time_ns == 1_000_000_123)
    );
    assert!(
        matches!(&records[1], Record::Messages(messages) if messages == &vec![Text::from("hi"), Text::from("hello")])
    );
    assert!(matches!(&records[2], Record::Tools(tools) if tools == &vec![Text::from("search")]));
    assert!(
        matches!(&records[3], Record::End {child: false, status: Status::Ok, time: 2_000_000_456, response: outputs} if outputs == &response())
    );
}

#[test]
fn streamed_chunks_share_span_and_final_chunk_cleans_up() {
    let (logger, state) = fixture(None, true, 8);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(false),
        response: &chunk(),
        start,
        end,
    });
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(true),
        response: &chunk(),
        start,
        end,
    });
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(false),
        response: &chunk(),
        start,
        end,
    });
    let records = &state.lock().unwrap().records;
    assert_eq!(records.len(), 7);
    assert!(matches!(&records[0], Record::Start(_, None)));
    assert!(
        matches!(&records[1], Record::Event(Event::StreamingChunk {delta}) if delta == "\"<bound method BaseModel.model_dump of Delta(content='\\u00e9\\ud83d\\ude00')>\"")
    );
    assert!(matches!(
        &records[2],
        Record::Event(Event::StreamingChunk { .. })
    ));
    assert!(
        matches!(&records[4], Record::End {status: Status::Ok, response: outputs, ..} if outputs == &response())
    );
    assert!(matches!(&records[5], Record::Start(_, None)));
}

#[test]
fn null_final_response_does_not_end_stream() {
    let (logger, state) = fixture(None, true, 8);
    let (start, end) = times();
    let mut kwargs = stream_kwargs(false);
    kwargs.complete_streaming_response = None;
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &Response::default(),
        start,
        end,
    });
    assert_eq!(state.lock().unwrap().records.len(), 1);
}

#[rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
fn final_response_keys(#[case] asynchronous: bool) {
    let (logger, state) = fixture(None, true, 8);
    let (start, end) = times();
    let mut kwargs = stream_kwargs(false);
    if asynchronous {
        kwargs.async_complete_response = Some(response());
    } else {
        kwargs.complete_streaming_response = Some(response());
    }
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &chunk(),
        start,
        end,
    });
    assert!(matches!(
        state.lock().unwrap().records.last(),
        Some(Record::End {
            status: Status::Ok,
            ..
        })
    ));
}

#[test]
fn sync_final_response_wins_when_both_are_present() {
    let (logger, state) = fixture(None, true, 8);
    let mut kwargs = stream_kwargs(true);
    kwargs.async_complete_response = Some(Response::default());
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &chunk(),
        start,
        end,
    });
    assert!(
        matches!(state.lock().unwrap().records.last(), Some(Record::End { response: outputs, .. }) if outputs == &response())
    );
}

#[rstest]
#[case::completion(Some("completion"), SpanKind::Llm)]
#[case::acompletion(Some("acompletion"), SpanKind::Llm)]
#[case::embeddings(Some("embeddings"), SpanKind::Embedding)]
#[case::singular(Some("embedding"), SpanKind::Llm)]
#[case::other(Some("rerank"), SpanKind::Llm)]
#[case::missing(None, SpanKind::Llm)]
fn span_type_and_parent(#[case] call_type: Option<&str>, #[case] kind: SpanKind) {
    let parent = Span {
        request_id: "active-trace".to_owned(),
        span_id: "active-span".to_owned(),
        parent_id: None,
    };
    let (logger, state) = fixture(Some(parent.clone()), true, 8);
    let mut kwargs = request(&[]);
    kwargs.call_type = call_type.map(str::to_owned);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &Response::default(),
        start,
        end,
    });
    let records = &state.lock().unwrap().records;
    assert!(
        matches!(&records[0], Record::Start(request, Some(active)) if active == &parent && request.kind == kind && request.name == format!("litellm-{}", call_type.unwrap_or("completion")) && request.tags.is_empty())
    );
    assert!(matches!(&records[1], Record::End { child: true, .. }));
}

#[test]
fn pops_only_five_optional_keys_and_uses_truthiness() {
    let mut kwargs = request(&[]);
    kwargs.tools = Some(vec![Text::from("top-tool")]);
    kwargs.optional_params.tools = Some(vec![Text::from("override")]);
    kwargs.optional_params.functions = Some(vec![Text::from("function")]);
    kwargs.optional_params.stream = Some(false);
    kwargs.optional_params.tool_choice = Some(Text::default());
    kwargs.optional_params.user = Some(String::new());
    kwargs
        .optional_params
        .remaining
        .insert("temperature".to_owned(), Text::from("0.5"));
    kwargs.prediction = Some(Text::from("prediction"));
    let inputs = kwargs.construct_input();
    assert_eq!(inputs.tools, Some(vec![Text::from("override")]));
    assert_eq!(inputs.functions, Some(vec![Text::from("function")]));
    assert_eq!(inputs.prediction, Some(Text::from("prediction")));
    assert_eq!(inputs.stream, None);
    assert_eq!(inputs.tool_choice, None);
    assert_eq!(inputs.user, None);
    assert!(kwargs.optional_params.stream.is_none());
    assert!(kwargs.optional_params.tool_choice.is_none());
    assert!(kwargs.optional_params.user.is_none());
    assert_eq!(
        kwargs.optional_params.remaining.get("temperature"),
        Some(&Text::from("0.5"))
    );
    let next = kwargs.construct_input();
    assert_eq!(next.tools, Some(vec![Text::from("top-tool")]));
    assert_eq!(next.functions, None);
}

#[test]
fn truthy_optional_values_and_empty_override() {
    let mut kwargs = request(&[]);
    kwargs.tools = Some(vec![Text::from("top")]);
    kwargs.optional_params.tools = Some(vec![]);
    kwargs.optional_params.stream = Some(true);
    kwargs.optional_params.tool_choice = Some(Text::from("auto"));
    kwargs.optional_params.user = Some("user".to_owned());
    let inputs = kwargs.construct_input();
    assert_eq!(inputs.tools, Some(vec![Text::from("top")]));
    assert_eq!(inputs.stream, Some(true));
    assert_eq!(inputs.tool_choice, Some(Text::from("auto")));
    assert_eq!(inputs.user.as_deref(), Some("user"));
}

#[test]
fn standard_attributes_are_exclusive_and_tags_come_from_them() {
    let (logger, state) = fixture(None, true, 8);
    let mut kwargs = request(&[]);
    kwargs.cache_hit = Some(true);
    kwargs.litellm_params.api_base = Some("fallback".to_owned());
    let standard = Standard {
        api_base: None,
        cache_hit: None,
        prompt_tokens: Some(2),
        completion_tokens: Some(3),
        total_tokens: Some(5),
        response: Some(Text::from("raw")),
        response_cost: None,
        saved_cache_cost: None,
        request_tags: vec![" team : core:api ".to_owned(), " audit ".to_owned()],
    };
    kwargs.standard_logging_object = Some(standard);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &Response::default(),
        start,
        end,
    });
    let records = &state.lock().unwrap().records;
    let Record::Start(request, _) = &records[0] else {
        panic!("start");
    };
    assert_eq!(
        request.tags.get("team").map(String::as_str),
        Some("core:api")
    );
    assert_eq!(request.tags.get("audit").map(String::as_str), Some(""));
    assert!(
        matches!(&request.attributes.details, Details::Standard { api_base: None, cache_hit: None, usage, raw_llm_response: Some(raw), .. } if usage.input_tokens == Some(2) && usage.output_tokens == Some(3) && usage.total_tokens == Some(5) && raw == &Text::from("raw"))
    );
    let encoded = serde_json::to_string(&request.attributes).unwrap();
    assert!(encoded.contains("\"raw_llm_response\":\"raw\""));
    assert!(encoded.contains("\"mlflow.chat.tokenUsage\""));
    assert!(!encoded.contains("custom_llm_provider"));
}

#[test]
fn fallback_attributes_preserve_kwargs_and_decimal_cost() {
    let mut kwargs = request(&[]);
    kwargs.model = Some("model".to_owned());
    kwargs.cache_hit = Some(false);
    kwargs.custom_llm_provider = Some("provider".to_owned());
    kwargs.litellm_params.api_base = Some("base".to_owned());
    kwargs.response_cost = Some(rust_decimal::Decimal::new(123, 4));
    let attributes = kwargs.extract_attributes();
    assert_eq!(attributes.model.as_deref(), Some("model"));
    let serialized = serde_json::to_string(&attributes).unwrap();
    assert!(serialized.contains("\"response_cost\":0.0123"));
    assert!(
        matches!(attributes.details, Details::Fallback {api_base: Some(base), cache_hit: Some(false), custom_llm_provider: Some(provider), response_cost: Some(cost)} if base == "base" && provider == "provider" && cost == rust_decimal::Decimal::new(123, 4))
    );
}

#[test]
fn duplicate_and_standalone_tags_match_python() {
    let tags = vec![
        "a: first".to_owned(),
        "a: last".to_owned(),
        " two words ".to_owned(),
        ": empty-key".to_owned(),
    ];
    assert_eq!(
        transform_tags(&tags),
        BTreeMap::from([
            ("a".to_owned(), "last".to_owned()),
            ("two words".to_owned(), String::new()),
            (String::new(), "empty-key".to_owned())
        ])
    );
}

#[rstest]
#[case::success(Status::Ok)]
#[case::failure(Status::Error)]
fn async_callbacks_match_sync(#[case] status: Status) {
    let (logger, state) = fixture(None, true, 8);
    let (start, end) = times();
    let mut kwargs = request(&[]);
    kwargs.exception = Some(ExceptionInfo {
        exception_type: "RuntimeError".to_owned(),
        message: "failed".to_owned(),
        stacktrace: "stack".to_owned(),
    });
    let mut original = kwargs.clone();
    let future = match status {
        Status::Ok => logger.async_log_success_event(Call {
            kwargs: &mut kwargs,
            response: &response(),
            start,
            end,
        }),
        Status::Error => logger.async_log_failure_event(Call {
            kwargs: &mut kwargs,
            response: &response(),
            start,
            end,
        }),
    };
    assert_eq!(
        pin!(future).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(())
    );
    let (synchronous, sync_state) = fixture(None, true, 8);
    let call = Call {
        kwargs: &mut original,
        response: &response(),
        start,
        end,
    };
    match status {
        Status::Ok => synchronous.log_success_event(call),
        Status::Error => synchronous.log_failure_event(call),
    }
    assert_eq!(
        state.lock().unwrap().records,
        sync_state.lock().unwrap().records
    );
    let records = &state.lock().unwrap().records;
    assert!(
        matches!(records.last(), Some(Record::End {status: recorded, ..}) if recorded == &status)
    );
    if status == Status::Error {
        assert!(
            matches!(&records[1], Record::Event(Event::Exception(info)) if info.message == "failed" && info.exception_type == "RuntimeError" && info.stacktrace == "stack")
        );
    }
}

#[rstest]
#[case::start(Stage::Start)]
#[case::active(Stage::Active)]
#[case::exception(Stage::Event)]
#[case::chat(Stage::Messages)]
#[case::tools(Stage::Tools)]
#[case::end(Stage::End)]
fn failures_are_swallowed(#[case] stage: Stage) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = subscriber::set_default(subscriber);
    let (logger, state) = fixture(None, true, 8);
    state.lock().unwrap().fail = Some(stage);
    let mut kwargs = request(&[]);
    kwargs.tools = Some(vec![Text::from("tool")]);
    kwargs.exception = Some(ExceptionInfo {
        exception_type: "Error".to_owned(),
        message: "error".to_owned(),
        stacktrace: String::new(),
    });
    let (start, end) = times();
    logger.log_failure_event(Call {
        kwargs: &mut kwargs,
        response: &response(),
        start,
        end,
    });
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &response(),
        start,
        end,
    });
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("DEBUG") && output.contains("MLflow logging error"));
}

#[rstest]
#[case::chat(Stage::Messages)]
#[case::end(Stage::End)]
#[case::chunk(Stage::Event)]
fn final_cleanup_survives_failures(#[case] stage: Stage) {
    let (logger, state) = fixture(None, true, 8);
    state.lock().unwrap().fail = Some(stage);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(true),
        response: &chunk(),
        start,
        end,
    });
    state.lock().unwrap().fail = None;
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(false),
        response: &chunk(),
        start,
        end,
    });
    let records = &state.lock().unwrap().records;
    assert_eq!(
        records
            .iter()
            .filter(|entry| matches!(entry, Record::Start(..)))
            .count(),
        2
    );
    if stage == Stage::Event {
        assert!(
            records
                .iter()
                .any(|entry| matches!(entry, Record::End { .. }))
        );
    }
}

#[test]
fn missing_chat_helpers_skip_attributes() {
    let (logger, state) = fixture(None, false, 8);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut request(&[]),
        response: &chunk(),
        start,
        end,
    });
    let records = &state.lock().unwrap().records;
    assert_eq!(records.len(), 2);
    assert!(matches!(&records[1], Record::End { .. }));
}

#[test]
fn malformed_chunk_does_not_prevent_finalization() {
    let (logger, state) = fixture(None, true, 8);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(true),
        response: &response(),
        start,
        end,
    });
    assert!(matches!(
        state.lock().unwrap().records.last(),
        Some(Record::End { .. })
    ));
}

#[test]
fn concurrent_first_chunks_create_one_span() {
    let (logger, state) = fixture(None, true, 8);
    let barrier = Barrier::new(8);
    thread::scope(|scope| {
        let threads: Vec<_> = iter::repeat_with(|| {
            scope.spawn(|| {
                let (start, end) = times();
                barrier.wait();
                logger.log_success_event(Call {
                    kwargs: &mut stream_kwargs(false),
                    response: &chunk(),
                    start,
                    end,
                });
            })
        })
        .take(8)
        .collect();
        assert!(threads.into_iter().all(|thread| thread.join().is_ok()));
    });
    let records = &state.lock().unwrap().records;
    assert_eq!(
        records
            .iter()
            .filter(|entry| matches!(entry, Record::Start(..)))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|entry| matches!(entry, Record::Event(..)))
            .count(),
        8
    );
}

fn request(messages: &[&str]) -> Kwargs<Text> {
    let inputs = messages.iter().map(|text| Text::from(*text)).collect();
    Kwargs {
        litellm_call_id: None,
        call_type: None,
        model: None,
        stream: false,
        messages: Some(inputs),
        tools: None,
        prediction: None,
        optional_params: OptionalParams::default(),
        standard_logging_object: None,
        cache_hit: None,
        custom_llm_provider: None,
        litellm_params: ProviderParams::default(),
        response_cost: None,
        exception: None,
        complete_streaming_response: None,
        async_complete_response: None,
    }
}

#[test]
fn missing_input_messages_abort_chat_extraction() {
    let (logger, state) = fixture(None, true, 8);
    let (start, end) = times();
    let mut kwargs = request(&[]);
    kwargs.messages = None;
    logger.log_success_event(Call {
        kwargs: &mut kwargs,
        response: &response(),
        start,
        end,
    });
    assert_eq!(state.lock().unwrap().records.len(), 1);
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("lock"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn stream_capacity_rejects_new_ids_and_finalization_frees_space() {
    let (logger, state) = fixture(None, true, 1);
    let (start, end) = times();
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(false),
        response: &chunk(),
        start,
        end,
    });
    let mut other = stream_kwargs(false);
    other.litellm_call_id = Some("other".to_owned());
    logger.log_success_event(Call {
        kwargs: &mut other,
        response: &chunk(),
        start,
        end,
    });
    assert_eq!(state.lock().unwrap().records.len(), 2);
    logger.log_success_event(Call {
        kwargs: &mut stream_kwargs(true),
        response: &chunk(),
        start,
        end,
    });
    logger.log_success_event(Call {
        kwargs: &mut other,
        response: &chunk(),
        start,
        end,
    });
    assert_eq!(
        state
            .lock()
            .unwrap()
            .records
            .iter()
            .filter(|entry| matches!(entry, Record::Start(..)))
            .count(),
        2
    );
}
