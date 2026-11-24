#![cfg(test)]

use std::future::Future;
use std::iter::from_fn;
use std::ptr::{eq, from_ref};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::fields::status::LoggingStatus;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::controls::call_params::TRUSTED_CREDENTIALS_KEY;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset};
use rstest::rstest;
use secrecy::ExposeSecret;
use serde_json::Number;
use tokio::runtime::Builder;

use super::handler::{
    StreamingParamsCall, StreamingParamsError, resolve_streaming_params, should_stream_file_content,
};
use super::logging::{CallbackFailure, FailureCall, FileStreamLogger, HiddenField, SuccessCall};
use super::source::{
    AsyncByteSource, CloseCall, CloseResult, EmptySource, SourceFailure, StreamError, StreamSource,
    SyncByteSource,
};
use super::{FileContentStreamingResponse, FileStreamSetup};

fn time(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).unwrap()
}

fn clock_time() -> DateTime<FixedOffset> {
    time("2026-01-01T00:00:05+00:00")
}

fn broken(message: &str) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

struct FixedClock(DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

fn clock() -> Arc<dyn SuccessClock> {
    Arc::new(FixedClock(clock_time()))
}

fn block_on<Output, F>(future: F) -> Output
where
    F: Future<Output = Output>,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[derive(Debug, Clone)]
enum Call {
    Success(SuccessCall),
    AsyncSuccess,
    SyncCallbacks(SuccessCall),
    Failure(FailureCall),
    AsyncFailure,
}

#[derive(Default)]
struct Counters {
    closes: Arc<AtomicUsize>,
    acloses: Arc<AtomicUsize>,
    awaited: Arc<AtomicUsize>,
}

struct RecordingLogger {
    start: Option<DateTime<FixedOffset>>,
    details_api_base: Mutex<Option<String>>,
    base_model: String,
    base_api_base: String,
    base_provider: Option<String>,
    payload: bool,
    fail_success: Option<String>,
    calls: Mutex<Vec<Call>>,
    builds: AtomicUsize,
}

impl RecordingLogger {
    fn new(base_model: &str, base_api_base: &str, base_provider: Option<&str>) -> Self {
        Self {
            start: None,
            details_api_base: Mutex::new(None),
            base_model: base_model.to_owned(),
            base_api_base: base_api_base.to_owned(),
            base_provider: base_provider.map(str::to_owned),
            payload: true,
            fail_success: None,
            calls: Mutex::new(Vec::new()),
            builds: AtomicUsize::new(0),
        }
    }

    fn with_start(mut self, start: Option<DateTime<FixedOffset>>) -> Self {
        self.start = start;
        self
    }

    fn with_api_base(self, api_base: Option<&str>) -> Self {
        *self.details_api_base.lock().unwrap() = api_base.map(str::to_owned);
        self
    }

    fn without_payload(mut self) -> Self {
        self.payload = false;
        self
    }

    fn failing_success(mut self, message: &str) -> Self {
        self.fail_success = Some(message.to_owned());
        self
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn build_count(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl FileStreamLogger for RecordingLogger {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        self.start
    }

    fn details_api_base(&self) -> Option<String> {
        self.details_api_base.lock().unwrap().clone()
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let _count = self.builds.fetch_add(1, Ordering::SeqCst);
        if !self.payload {
            return None;
        }
        Some(payload_fixture(
            "f",
            &self.base_model,
            &self.base_api_base,
            self.base_provider.as_deref(),
        ))
    }

    fn success_handler(&self, call: SuccessCall) -> Result<(), CallbackFailure> {
        if let Some(message) = self.fail_success.clone() {
            return Err(CallbackFailure { message });
        }
        self.record(Call::Success(call));
        Ok(())
    }

    async fn async_success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        if let Some(message) = self.fail_success.clone() {
            return Err(CallbackFailure { message });
        }
        self.record(Call::AsyncSuccess);
        Ok(())
    }

    fn success_sync_callbacks(&self, call: SuccessCall) -> Result<(), CallbackFailure> {
        self.record(Call::SyncCallbacks(call));
        Ok(())
    }

    fn failure_handler(&self, call: FailureCall) -> Result<(), CallbackFailure> {
        self.record(Call::Failure(call));
        Ok(())
    }

    async fn async_failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        self.record(Call::AsyncFailure);
        Ok(())
    }
}

fn payload_fixture(
    id: &str,
    model: &str,
    api_base: &str,
    provider: Option<&str>,
) -> StandardLoggingPayload {
    let provider_json = provider.map_or_else(|| String::from("null"), |text| format!("\"{text}\""));
    let text = format!(
        r#"{{"id": "{id}", "trace_id": "trace-1", "session_id": "session-1", "call_type": "afile_content",
        "response_cost": 0.0, "status": "success", "status_fields": {{"llm_api_status": "success", "guardrail_status": "not_run"}},
        "total_tokens": 0, "prompt_tokens": 0, "completion_tokens": 0,
        "startTime": 1767225600.0, "endTime": 1767225601.0, "completionStartTime": 1767225600.0,
        "response_time": 1.0, "model_map_information": {{}}, "model": "{model}", "api_base": "{api_base}",
        "custom_llm_provider": {provider_json}, "metadata": {{}}, "saved_cache_cost": 0.0, "request_tags": [],
        "model_parameters": {{}}, "hidden_params": {{}}, "response": {{"id": "{id}", "object": "file.content"}}}}"#
    );
    serde_json::from_str(&text).unwrap()
}

#[derive(Clone)]
enum AsyncClose {
    Absent,
    Returned,
    Awaitable(CloseResult),
}

struct Scripted {
    items: Vec<Result<Vec<u8>, SourceFailure>>,
    index: usize,
    counters: Counters,
    close_result: Option<CloseResult>,
    aclose_result: Option<CloseResult>,
    close_mode: AsyncClose,
    async_mode: bool,
}

impl Scripted {
    fn new(items: Vec<Result<Vec<u8>, SourceFailure>>) -> Self {
        Self {
            items,
            index: 0,
            counters: Counters::default(),
            close_result: None,
            aclose_result: None,
            close_mode: AsyncClose::Absent,
            async_mode: false,
        }
    }

    fn asynchronous(mut self) -> Self {
        self.async_mode = true;
        self
    }

    fn into_source(self) -> StreamSource {
        if self.async_mode {
            StreamSource::Async(Box::new(self))
        } else {
            StreamSource::Sync(Box::new(self))
        }
    }

    fn with_sync_close(mut self, result: Option<CloseResult>) -> Self {
        self.close_result = result;
        self
    }

    fn with_aclose(mut self, result: CloseResult) -> Self {
        self.aclose_result = Some(result);
        self
    }

    fn with_close(mut self, mode: AsyncClose) -> Self {
        self.close_mode = mode;
        self
    }

    fn counters(&self) -> Counters {
        Counters {
            closes: Arc::clone(&self.counters.closes),
            acloses: Arc::clone(&self.counters.acloses),
            awaited: Arc::clone(&self.counters.awaited),
        }
    }

    fn pull(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        let item = self.items.get(self.index)?.clone();
        self.index = self.index.saturating_add(1);
        Some(item)
    }
}

impl SyncByteSource for Scripted {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        self.pull()
    }

    fn close(&mut self) -> Option<CloseResult> {
        let _count = self.counters.closes.fetch_add(1, Ordering::SeqCst);
        self.close_result.clone()
    }
}

#[async_trait]
impl AsyncByteSource for Scripted {
    async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        self.pull()
    }

    async fn aclose(&mut self) -> Option<CloseResult> {
        let result = self.aclose_result.clone()?;
        let _count = self.counters.acloses.fetch_add(1, Ordering::SeqCst);
        Some(result)
    }

    fn close(&mut self) -> CloseCall<'_> {
        let _count = self.counters.closes.fetch_add(1, Ordering::SeqCst);
        match &self.close_mode {
            AsyncClose::Absent => CloseCall::Absent,
            AsyncClose::Returned => CloseCall::Returned,
            AsyncClose::Awaitable(result) => {
                let pending = result.clone();
                let awaited = Arc::clone(&self.counters.awaited);
                CloseCall::Awaitable(Box::pin(async move {
                    let _awaited_count = awaited.fetch_add(1, Ordering::SeqCst);
                    pending
                }))
            }
        }
    }
}

fn setup(source: StreamSource, model: Option<&str>, provider: Option<&str>) -> FileStreamSetup {
    FileStreamSetup {
        source,
        file_id: String::from("f"),
        model: model.map(str::to_owned),
        custom_llm_provider: provider.map(str::to_owned),
        logger: None,
    }
}

fn file_stream(
    source: Scripted,
    model: Option<&str>,
    provider: Option<&str>,
    logger: Option<Arc<dyn FileStreamLogger>>,
) -> FileContentStreamingResponse {
    let mut stream = setup(source.into_source(), model, provider);
    stream.logger = logger;
    FileContentStreamingResponse::new(stream, clock())
}

fn chunks(items: &[&[u8]]) -> Vec<Result<Vec<u8>, SourceFailure>> {
    items.iter().map(|item| Ok(item.to_vec())).collect()
}

fn drain_sync(stream: &mut FileContentStreamingResponse) -> Vec<Vec<u8>> {
    let mut iter = stream.sync_iter().unwrap();
    from_fn(|| iter.next_chunk())
        .map(|item| item.unwrap())
        .collect()
}

fn drain_async(stream: &mut FileContentStreamingResponse) -> Vec<Vec<u8>> {
    block_on(async {
        let mut iter = stream.async_iter().unwrap();
        let mut drained = Vec::new();
        while let Some(item) = iter.next_chunk().await {
            drained.push(item.unwrap());
        }
        drained
    })
}

fn expected_response(model: Option<&str>) -> JsonObject {
    let mut response = JsonObject::new();
    response.insert(String::from("id"), Json::Text(String::from("f")));
    response.insert(
        String::from("object"),
        Json::Text(String::from("file.content")),
    );
    if let Some(name) = model {
        response.insert(String::from("model"), Json::Text(name.to_owned()));
    }
    response
}

#[test]
fn streaming_projection_routes_credentials() {
    let mut data = JsonObject::new();
    data.insert(String::from("model"), Json::Text(String::from("m")));
    data.insert(String::from("file_id"), Json::Text(String::from("encoded")));
    data.insert(String::from("x"), Json::Number(Number::from(1_i32)));
    let mut credentials = JsonObject::new();
    credentials.insert(
        String::from("custom_llm_provider"),
        Json::Text(String::from("openai")),
    );
    credentials.insert(String::from("api_key"), Json::Text(String::from("k")));
    let resolved = resolve_streaming_params(&StreamingParamsCall {
        custom_llm_provider: "openai",
        file_id: "encoded",
        data: &data,
        should_route: true,
        original_file_id: Some("f"),
        credentials: Some(&credentials),
    })
    .unwrap();
    assert_eq!(resolved.provider, "openai");
    assert_eq!(resolved.file_id, "f");
    assert_eq!(resolved.data.get("model"), None);
    assert_eq!(
        resolved.data.get("file_id"),
        Some(&Json::Text(String::from("f")))
    );
    assert_eq!(
        resolved.data.get("x"),
        Some(&Json::Number(Number::from(1_i32)))
    );
    assert!(!resolved.data.contains_key(TRUSTED_CREDENTIALS_KEY));
    let trusted = resolved.trusted_credentials.as_ref().unwrap();
    let api_key = trusted.values.get("api_key");
    assert_eq!(api_key.map(ExposeSecret::expose_secret), Some("k"));
    let provider = trusted.values.get("custom_llm_provider");
    assert_eq!(provider.map(ExposeSecret::expose_secret), Some("openai"));
    assert_eq!(data.get("model"), Some(&Json::Text(String::from("m"))));
    assert!(!data.contains_key("api_key"));
}

#[rstest]
#[case::not_routed(false, true)]
#[case::no_credentials(true, false)]
fn streaming_projection_leaves_unrouted_input_alone(
    #[case] should_route: bool,
    #[case] with_credentials: bool,
) {
    let mut data = JsonObject::new();
    data.insert(String::from("model"), Json::Text(String::from("m")));
    data.insert(String::from("file_id"), Json::Text(String::from("encoded")));
    let mut credentials = JsonObject::new();
    credentials.insert(
        String::from("custom_llm_provider"),
        Json::Text(String::from("openai")),
    );
    let resolved = resolve_streaming_params(&StreamingParamsCall {
        custom_llm_provider: "openai",
        file_id: "f",
        data: &data,
        should_route,
        original_file_id: Some("f"),
        credentials: with_credentials.then_some(&credentials),
    })
    .unwrap();
    assert_eq!(resolved.provider, "openai");
    assert_eq!(resolved.file_id, "f");
    assert_eq!(resolved.data, data);
    assert!(resolved.trusted_credentials.is_none());
}

#[rstest]
#[case::missing_provider(None, StreamingParamsError::MissingProvider)]
#[case::provider_not_text(Some(Json::Bool(true)), StreamingParamsError::ProviderNotText)]
fn streaming_projection_rejects_a_bad_credential_provider(
    #[case] provider: Option<Json>,
    #[case] expected: StreamingParamsError,
) {
    let mut credentials = JsonObject::new();
    credentials.insert(String::from("api_key"), Json::Text(String::from("k")));
    if let Some(text) = provider {
        credentials.insert(String::from("custom_llm_provider"), text);
    }
    let mut data = JsonObject::new();
    data.insert(String::from("model"), Json::Text(String::from("m")));
    let Err(failure) = resolve_streaming_params(&StreamingParamsCall {
        custom_llm_provider: "openai",
        file_id: "f",
        data: &data,
        should_route: true,
        original_file_id: Some("f"),
        credentials: Some(&credentials),
    }) else {
        panic!("the bad credential provider resolved");
    };
    assert_eq!(failure, expected);
}

#[rstest]
#[case::missing_file_id(None, StreamingParamsError::MissingFileId)]
#[case::file_id_not_text(Some(Json::Bool(true)), StreamingParamsError::FileIdNotText)]
fn streaming_projection_rejects_a_bad_file_id(
    #[case] file_id: Option<Json>,
    #[case] expected: StreamingParamsError,
) {
    let mut credentials = JsonObject::new();
    credentials.insert(
        String::from("custom_llm_provider"),
        Json::Text(String::from("openai")),
    );
    let mut data = JsonObject::new();
    if let Some(value) = file_id {
        data.insert(String::from("file_id"), value);
    }
    let Err(failure) = resolve_streaming_params(&StreamingParamsCall {
        custom_llm_provider: "openai",
        file_id: "f",
        data: &data,
        should_route: true,
        original_file_id: None,
        credentials: Some(&credentials),
    }) else {
        panic!("the bad file id resolved");
    };
    assert_eq!(failure, expected);
}

#[rstest]
#[case::openai("openai", true)]
#[case::hosted_vllm("hosted_vllm", true)]
#[case::litellm_proxy("litellm_proxy", true)]
#[case::vertex_ai("vertex_ai", true)]
#[case::azure("azure", false)]
#[case::bedrock("bedrock", false)]
#[case::anthropic("anthropic", false)]
#[case::empty("", false)]
fn streaming_predicate_accepts_only_supported_providers(
    #[case] provider: &str,
    #[case] expected: bool,
) {
    assert_eq!(should_stream_file_content(Some(provider)), expected);
}

#[test]
fn streaming_predicate_rejects_a_null_provider() {
    assert!(!should_stream_file_content(None));
}

#[test]
fn hidden_stream_metadata_matches_litellm() {
    let logger = Arc::new(RecordingLogger::new("", "u", None).with_api_base(Some("u/")));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab"])),
        Some("m"),
        Some("openai"),
        Some(logger),
    );
    drain_sync(&mut stream);
    let payload = stream.logging.standard_logging_object.as_ref().unwrap();
    assert_eq!(payload.id, "f");
    assert_eq!(payload.model, "m");
    assert_eq!(payload.api_base, "u/");
    assert_eq!(payload.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(payload.status, LoggingStatus::Success);
    assert_eq!(payload.hidden_params.api_base.as_deref(), Some("u/"));
    assert_eq!(
        payload.hidden_params.litellm_model_name.as_deref(),
        Some("m")
    );
    assert!(payload.hidden_params.response_cost.is_none());
    assert_eq!(
        payload.response,
        Some(Json::Object(expected_response(Some("m"))))
    );
}

#[test]
fn hidden_stream_metadata_omits_response_model_for_an_empty_model() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab"])),
        Some(""),
        None,
        Some(logger),
    );
    drain_sync(&mut stream);
    let payload = stream.logging.standard_logging_object.as_ref().unwrap();
    assert_eq!(payload.model, "");
    assert_eq!(
        payload.response,
        Some(Json::Object(expected_response(None)))
    );
    assert_eq!(
        payload.hidden_params.litellm_model_name.as_deref(),
        Some("")
    );
}

#[test]
fn hidden_stream_metadata_syncs_api_base_once() {
    let logger = Arc::new(RecordingLogger::new("", "", None).with_api_base(Some("u")));
    let mut stream = file_stream(
        Scripted::new(Vec::new()),
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    assert_eq!(
        stream.logging.hidden.api_base,
        HiddenField::Value(String::from("u"))
    );
    *logger.details_api_base.lock().unwrap() = Some(String::from("other"));
    stream.logging.sync_hidden_params();
    assert_eq!(
        stream.logging.hidden.api_base,
        HiddenField::Value(String::from("u"))
    );
    let payload = stream.logging.standard_object(clock_time()).unwrap();
    assert_eq!(payload.api_base, "u");
    assert!(payload.hidden_params.response_cost.is_none());
}

#[test]
fn hidden_stream_metadata_ignores_an_empty_api_base() {
    let logger = Arc::new(RecordingLogger::new("", "base", None).with_api_base(Some("")));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab"])),
        Some("m"),
        None,
        Some(logger),
    );
    drain_sync(&mut stream);
    let payload = stream.logging.standard_logging_object.as_ref().unwrap();
    assert_eq!(payload.api_base, "base");
    assert!(payload.hidden_params.api_base.is_none());
}

#[test]
fn hidden_stream_metadata_caches_an_existing_payload() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(Vec::new()),
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    let first = from_ref(stream.logging.standard_object(clock_time()).unwrap());
    let second = from_ref(
        stream
            .logging
            .standard_object(time("2026-01-01T00:00:09+00:00"))
            .unwrap(),
    );
    assert!(eq(first, second));
    assert_eq!(logger.build_count(), 1);
}

#[test]
fn hidden_stream_metadata_without_a_logger_has_no_payload() {
    let mut stream = file_stream(Scripted::new(chunks(&[b"ab"])), None, None, None);
    assert!(stream.logging.standard_object(clock_time()).is_none());
    drain_sync(&mut stream);
    assert!(stream.logging.standard_logging_object.is_none());
    assert!(!stream.logging.logging_completed);
}

#[test]
fn hidden_stream_metadata_without_a_built_payload_has_none() {
    let logger = Arc::new(RecordingLogger::new("", "", None).without_payload());
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab"])),
        Some("m"),
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    assert!(stream.logging.standard_object(clock_time()).is_none());
    assert_eq!(drain_sync(&mut stream), vec![b"ab".to_vec()]);
    assert!(stream.logging.logging_completed);
    assert!(stream.logging.standard_logging_object.is_none());
    assert_eq!(logger.build_count(), 2);
    let calls = logger.calls();
    assert_eq!(calls.len(), 1);
    let Some(Call::Success(call)) = calls.first() else {
        panic!("the one callback was not a sync success");
    };
    assert!(call.standard_logging_object.is_none());
}

#[test]
fn stream_success_once_logs_one_sync_callback() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab", b"cd"])),
        Some("m"),
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    assert_eq!(
        drain_sync(&mut stream),
        vec![b"ab".to_vec(), b"cd".to_vec()]
    );
    assert!(stream.logging.logging_completed);
    let mut iter = stream.sync_iter().unwrap();
    assert!(iter.next_chunk().is_none());
    let calls = logger.calls();
    assert_eq!(calls.len(), 1);
    let Some(Call::Success(call)) = calls.first() else {
        panic!("the one callback was not a sync success");
    };
    assert_eq!(call.result, Json::Object(expected_response(Some("m"))));
    assert_eq!(call.start_time, clock_time());
    assert_eq!(call.end_time, clock_time());
    assert!(call.standard_logging_object.is_some());
}

#[test]
fn stream_success_once_logs_async_then_sync_callbacks() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab", b"cd"])).asynchronous(),
        Some("m"),
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    assert_eq!(
        drain_async(&mut stream),
        vec![b"ab".to_vec(), b"cd".to_vec()]
    );
    let calls = logger.calls();
    assert_eq!(calls.len(), 2);
    assert!(matches!(calls.first(), Some(Call::AsyncSuccess)));
    let Some(Call::SyncCallbacks(sync_call)) = calls.get(1) else {
        panic!("the second callback was not the sync one");
    };
    assert!(sync_call.standard_logging_object.is_none());
}

#[test]
fn stream_success_once_uses_the_logger_or_the_clock_start_time() {
    let logger = Arc::new(
        RecordingLogger::new("", "", None).with_start(Some(time("2026-01-01T00:00:00+00:00"))),
    );
    let from_logger = file_stream(Scripted::new(Vec::new()), None, None, Some(logger));
    assert_eq!(
        from_logger.logging.start_time,
        time("2026-01-01T00:00:00+00:00")
    );
    let from_clock = file_stream(Scripted::new(Vec::new()), None, None, None);
    assert_eq!(from_clock.logging.start_time, clock_time());
}

#[test]
fn stream_success_once_propagates_callback_errors_without_a_second_log() {
    let logger = Arc::new(RecordingLogger::new("", "", None).failing_success("callback broke"));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab"])),
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    let mut iter = stream.sync_iter().unwrap();
    assert_eq!(iter.next_chunk().unwrap().unwrap(), b"ab".to_vec());
    let failure = iter.next_chunk().unwrap().unwrap_err();
    assert!(
        matches!(&failure, StreamError::Callback { failure: callback } if callback.message == "callback broke")
    );
    assert!(iter.next_chunk().is_none());
    assert!(stream.logging.logging_completed);
    assert_eq!(logger.calls().len(), 0);
}

#[test]
fn stream_success_once_propagates_async_callback_errors_without_a_second_log() {
    let logger = Arc::new(RecordingLogger::new("", "", None).failing_success("callback broke"));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"ab"])).asynchronous(),
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    let failure = block_on(async {
        let mut iter = stream.async_iter().unwrap();
        assert_eq!(iter.next_chunk().await.unwrap().unwrap(), b"ab".to_vec());
        iter.next_chunk().await.unwrap().unwrap_err()
    });
    assert!(
        matches!(&failure, StreamError::Callback { failure: callback } if callback.message == "callback broke")
    );
    assert!(block_on(async { stream.async_iter().unwrap().next_chunk().await }).is_none());
    assert!(stream.logging.logging_completed);
    assert_eq!(logger.calls().len(), 0);
}

#[test]
fn stream_success_once_never_puts_file_bytes_in_the_payload() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(chunks(&[b"secret bytes"])),
        None,
        None,
        Some(logger),
    );
    drain_sync(&mut stream);
    let payload = stream.logging.standard_logging_object.as_ref().unwrap();
    assert_eq!(
        payload.response,
        Some(Json::Object(expected_response(None)))
    );
    assert!(payload.messages.is_none());
}

#[test]
fn stream_failure_once_logs_one_sync_failure() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(vec![
            Ok(b"a".to_vec()),
            Err(SourceFailure::Exception(broken("broken"))),
        ]),
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    let mut iter = stream.sync_iter().unwrap();
    assert_eq!(iter.next_chunk().unwrap().unwrap(), b"a".to_vec());
    let failure = iter.next_chunk().unwrap().unwrap_err();
    let StreamError::Source { failure: exception } = &failure else {
        panic!("the original error was not rethrown");
    };
    assert_eq!(exception, &broken("broken"));
    assert!(iter.next_chunk().is_none());
    let calls = logger.calls();
    assert_eq!(calls.len(), 1);
    let Some(Call::Failure(call)) = calls.first() else {
        panic!("the one callback was not a sync failure");
    };
    assert_eq!(call.exception, broken("broken"));
    assert_eq!(call.traceback, "");
    assert_eq!(call.start_time, clock_time());
    assert_eq!(call.end_time, clock_time());
}

#[test]
fn stream_failure_once_runs_the_sync_then_async_failure_handler() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(vec![
            Ok(b"a".to_vec()),
            Err(SourceFailure::Exception(broken("broken"))),
        ])
        .asynchronous(),
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    let failure = block_on(async {
        let mut iter = stream.async_iter().unwrap();
        assert_eq!(iter.next_chunk().await.unwrap().unwrap(), b"a".to_vec());
        iter.next_chunk().await.unwrap().unwrap_err()
    });
    assert!(matches!(&failure, StreamError::Source { .. }));
    let calls = logger.calls();
    assert_eq!(calls.len(), 2);
    assert!(matches!(calls.first(), Some(Call::Failure(_))));
    assert!(matches!(calls.get(1), Some(Call::AsyncFailure)));
}

#[test]
fn stream_failure_once_skips_callbacks_for_a_cancellation() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let mut stream = file_stream(
        Scripted::new(vec![Err(SourceFailure::Cancellation)]),
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    let mut iter = stream.sync_iter().unwrap();
    let failure = iter.next_chunk().unwrap().unwrap_err();
    assert!(matches!(&failure, StreamError::Cancelled));
    assert_eq!(logger.calls().len(), 0);
    assert!(!stream.logging.logging_completed);
}

#[test]
fn stream_failure_once_rejects_the_wrong_iteration_capability() {
    let mut async_only = file_stream(Scripted::new(Vec::new()).asynchronous(), None, None, None);
    let failure = async_only.sync_iter().err().unwrap();
    assert_eq!(
        failure.to_string(),
        "File content stream does not support sync iteration"
    );
    let mut sync_only = file_stream(Scripted::new(Vec::new()), None, None, None);
    let unsupported = sync_only.async_iter().err().unwrap();
    assert_eq!(
        unsupported.to_string(),
        "File content stream does not support async iteration"
    );
}

#[test]
fn explicit_close_suppresses_logging() {
    let logger = Arc::new(RecordingLogger::new("", "", None));
    let source = Scripted::new(chunks(&[b"ab", b"cd"])).with_sync_close(Some(Ok(())));
    let counters = source.counters();
    let mut stream = file_stream(
        source,
        None,
        None,
        Some(Arc::<RecordingLogger>::clone(&logger)),
    );
    let mut iter = stream.sync_iter().unwrap();
    assert_eq!(iter.next_chunk().unwrap().unwrap(), b"ab".to_vec());
    assert!(!stream.close_completed);
    stream.close().unwrap();
    assert!(stream.close_completed);
    assert!(stream.logging.logging_completed);
    assert_eq!(counters.closes.load(Ordering::SeqCst), 1);
    assert!(drain_sync(&mut stream).is_empty());
    assert!(logger.calls().is_empty());
    stream.close().unwrap();
    assert_eq!(counters.closes.load(Ordering::SeqCst), 1);
}

#[test]
fn explicit_close_calls_the_underlying_close_once_even_when_it_raises() {
    let source = Scripted::new(Vec::new()).with_sync_close(Some(Err(broken("cleanup broke"))));
    let counters = source.counters();
    let mut stream = file_stream(source, None, None, None);
    let failure = stream.close().unwrap_err();
    assert!(matches!(&failure, StreamError::Cleanup { .. }));
    assert_eq!(counters.closes.load(Ordering::SeqCst), 1);
    stream.close().unwrap();
    assert_eq!(counters.closes.load(Ordering::SeqCst), 1);
}

#[test]
fn explicit_close_prefers_aclose() {
    let source = Scripted::new(chunks(&[b"ab"]))
        .with_aclose(Ok(()))
        .with_close(AsyncClose::Awaitable(Ok(())));
    let counters = source.counters();
    let mut stream = file_stream(source.asynchronous(), None, None, None);
    block_on(stream.aclose()).unwrap();
    assert!(stream.close_completed);
    assert_eq!(counters.acloses.load(Ordering::SeqCst), 1);
    assert_eq!(counters.closes.load(Ordering::SeqCst), 0);
}

#[test]
fn explicit_close_awaits_a_non_null_close_result() {
    let source = Scripted::new(chunks(&[b"ab"]))
        .with_close(AsyncClose::Awaitable(Err(broken("close broke"))));
    let counters = source.counters();
    let mut stream = file_stream(source.asynchronous(), None, None, None);
    let failure = block_on(stream.aclose()).unwrap_err();
    assert!(matches!(&failure, StreamError::Cleanup { .. }));
    assert_eq!(counters.closes.load(Ordering::SeqCst), 1);
    assert_eq!(counters.awaited.load(Ordering::SeqCst), 1);
}

#[test]
fn explicit_close_sync_calls_only_close() {
    let source = Scripted::new(chunks(&[b"ab"]))
        .with_aclose(Ok(()))
        .with_close(AsyncClose::Awaitable(Ok(())));
    let counters = source.counters();
    let mut stream = file_stream(source.asynchronous(), None, None, None);
    stream.close().unwrap();
    assert!(stream.close_completed);
    assert_eq!(counters.acloses.load(Ordering::SeqCst), 0);
    assert_eq!(counters.closes.load(Ordering::SeqCst), 1);
    assert_eq!(counters.awaited.load(Ordering::SeqCst), 0);
}

#[test]
fn empty_source_has_no_close() {
    let mut source = EmptySource;
    assert!(source.close().is_none());
}

#[test]
fn explicit_close_makes_async_iteration_unsupported() {
    let source = Scripted::new(chunks(&[b"ab"])).with_aclose(Ok(()));
    let mut stream = file_stream(source.asynchronous(), None, None, None);
    block_on(stream.aclose()).unwrap();
    let failure = stream.async_iter().err().unwrap();
    assert!(matches!(&failure, StreamError::UnsupportedAsync));
    assert!(drain_sync(&mut stream).is_empty());
}

#[test]
fn explicit_close_treats_a_plain_close_result_as_done() {
    let source = Scripted::new(Vec::new()).with_close(AsyncClose::Returned);
    let counters = source.counters();
    let mut stream = file_stream(source.asynchronous(), None, None, None);
    block_on(stream.aclose()).unwrap();
    assert!(stream.close_completed);
    assert_eq!(counters.closes.load(Ordering::SeqCst), 1);
}
