#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use std::vec::IntoIter;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::interactions::contract::config::BaseInteractionsAPIConfig;
use betterlite_core::api::interactions::contract::error::InteractionsContractError;
use betterlite_core::api::interactions::contract::records::{
    CompleteUrlRequest, CreateInteractionRequest, EnvironmentRequest, InteractionIdRequest,
    InteractionsLogging, InteractionsUpstream, Params, PreparedRequest, RequestHeaders,
};
use betterlite_core::api::interactions::streaming::async_iterator::InteractionsStreamIterator;
use betterlite_core::api::interactions::streaming::error::{
    InteractionSourceError, InteractionStreamError,
};
use betterlite_core::api::interactions::streaming::logging::InteractionsStreamLogging;
use betterlite_core::api::interactions::streaming::records::{
    InteractionsApiBase, InteractionsStreamSetup, InteractionsSuccessCall, SharedApiBase,
};
use betterlite_core::api::interactions::streaming::response::{
    InteractionBufferedResponse, InteractionsAsyncResponse, InteractionsLineStream,
    InteractionsSyncResponse, MAX_LINE_BYTES,
};
use betterlite_core::api::interactions::streaming::sync_iterator::SyncInteractionsStreamIterator;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_interactions::types::response::{
    CancelInteractionResult, DeleteInteractionResult, InteractionsApiResponse,
    InteractionsStreamChunk,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::api_base_lookup::{
    ApiBaseContext, ApiBaseError, ApiBaseHooks, ApiBaseParams, LoginFreeProvider,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use chrono::{DateTime, Duration as Seconds, FixedOffset};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

const COMPLETED_STATUS: &str = r#"{"id":"int_1","status":"completed"}"#;
const COMPLETED_EVENT: &str = r#"{"id":"int_2","event_type":"interaction.completed"}"#;

#[tokio::test]
async fn async_iterator_construction_builds_hidden_params_from_the_setup() {
    let catalog = ModelCatalog::default();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = resolution_context(&catalog, &known, &custom, &environment);
    let aliases = BTreeMap::new();
    let hooks = NoHooks;
    let shared = SharedApiBase {
        context: ApiBaseContext {
            model_alias_map: &aliases,
            hooks: &hooks,
            resolution: &resolution,
        },
    };
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(alias_params("https://alias.test"));
    let mut response = FakeAsyncResponse::new(&[r#"data: {"id":"int_1"}"#]);
    response.headers.insert(
        String::from("x-ratelimit-remaining-requests"),
        String::from("9"),
    );
    let calls = Arc::clone(&response.calls);
    let model_metadata = metadata("model-1");
    let mut iterator = InteractionsStreamIterator::new(
        response,
        InteractionsStreamSetup {
            model: Some("gemini-2.5-flash"),
            config: &config,
            logging: &mut logging,
            litellm_metadata: Some(&model_metadata),
            custom_llm_provider: Some("gemini"),
            api_base: &shared,
        },
    )
    .expect("the fixture iterator builds");
    assert_eq!(
        iterator.base.hidden_params.model_id.as_deref(),
        Some("model-1")
    );
    assert_eq!(
        iterator.base.hidden_params.api_base.as_deref(),
        Some("https://alias.test")
    );
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("llm_provider-x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(iterator.base.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(iterator.base.custom_llm_provider.as_deref(), Some("gemini"));
    assert!(!iterator.base.finished);
    assert_eq!(iterator.base.completed_response, None);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drain_async(&mut iterator).await.len(), 1);
}

#[tokio::test]
async fn async_iterator_construction_resolves_the_api_base_from_the_model() {
    let catalog = ModelCatalog::default();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = resolution_context(&catalog, &known, &custom, &environment);
    let aliases = BTreeMap::new();
    let hooks = NoHooks;
    let shared = SharedApiBase {
        context: ApiBaseContext {
            model_alias_map: &aliases,
            hooks: &hooks,
            resolution: &resolution,
        },
    };
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[]),
        InteractionsStreamSetup {
            model: Some("gemini/gemini-2.5-flash"),
            config: &config,
            logging: &mut logging,
            litellm_metadata: None,
            custom_llm_provider: None,
            api_base: &shared,
        },
    )
    .expect("the fixture iterator builds");
    assert_eq!(
        iterator.base.hidden_params.api_base.as_deref(),
        Some(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        )
    );
    assert!(iterator.next_chunk().await.is_none());
}

#[tokio::test]
async fn async_iterator_construction_swallows_api_base_resolution_errors() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[]),
        InteractionsStreamSetup {
            model: Some("gemini-2.5-flash"),
            config: &config,
            logging: &mut logging,
            litellm_metadata: None,
            custom_llm_provider: None,
            api_base: &FailingApiBase,
        },
    )
    .expect("the fixture iterator builds");
    assert_eq!(iterator.base.hidden_params.api_base, None);
}

#[tokio::test]
async fn async_iterator_construction_without_metadata_leaves_model_id_empty() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    assert_eq!(iterator.base.hidden_params.model_id, None);
    assert_eq!(iterator.base.hidden_params.api_base, None);
    assert!(iterator.next_chunk().await.is_none());
}

#[test]
fn sync_iterator_construction_calls_iter_lines_once() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(litellm_params("https://sync.test"));
    let model_metadata = metadata("model-2");
    let response = FakeSyncResponse::new(&[r#"data: {"id":"int_1"}"#]);
    let calls = Arc::clone(&response.calls);
    let mut iterator = SyncInteractionsStreamIterator::new(
        response,
        InteractionsStreamSetup {
            model: Some("gemini-2.5-flash"),
            config: &config,
            logging: &mut logging,
            litellm_metadata: Some(&model_metadata),
            custom_llm_provider: None,
            api_base: &NO_API_BASE,
        },
    )
    .expect("the fixture iterator builds");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        iterator.base.hidden_params.model_id.as_deref(),
        Some("model-2")
    );
    assert_eq!(
        iterator.base.hidden_params.api_base.as_deref(),
        Some("https://sync.test")
    );
    assert_eq!(drain_sync(&mut iterator).len(), 1);
    assert!(iterator.base.finished);
}

#[test]
fn sync_iterator_construction_rejects_a_wrongly_typed_api_base() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::from([(
        String::from("litellm_params"),
        JsonValue::Object(IndexMap::from([(
            String::from("api_base"),
            JsonValue::Bool(true),
        )])),
    )]));
    let Err(error) = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&[]),
        InteractionsStreamSetup {
            model: None,
            config: &config,
            logging: &mut logging,
            litellm_metadata: None,
            custom_llm_provider: None,
            api_base: &NO_API_BASE,
        },
    ) else {
        panic!("a boolean api base is refused");
    };
    assert!(matches!(error, InteractionStreamError::FieldType(_)));
}

#[tokio::test]
async fn async_iterator_construction_calls_aiter_lines_once() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let response = FakeAsyncResponse::new(&[]);
    let calls = Arc::clone(&response.calls);
    let mut iterator =
        InteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the fixture iterator builds");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drain_async(&mut iterator).await.len(), 0);
    assert!(iterator.base.finished);
}

#[tokio::test]
async fn chunk_processing_strips_the_sse_data_prefix() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[r#"data: {"id":"a"}"#, r#"data:{"id":"b"}"#, r#"{"id":"c"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["a", "b", "c"]);
}

#[tokio::test]
async fn chunk_processing_ignores_blank_lines() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&["", r#"data: {"id":"a"}"#, "", "", r#"data: {"id":"b"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["a", "b"]);
}

#[tokio::test]
async fn chunk_processing_ignores_malformed_json() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&["data: not json", r#"data: {"id":"after"}"#, "data: {"]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["after"]);
}

#[tokio::test]
async fn chunk_processing_ignores_non_object_json() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            "data: [1, 2]",
            "data: \"text\"",
            "data: 7",
            "data: null",
            r#"data: {"id":"ok"}"#,
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["ok"]);
}

#[tokio::test]
async fn chunk_processing_treats_done_as_the_end() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            r#"data: {"id":"first"}"#,
            "data: [DONE]",
            r#"data: {"id":"after"}"#,
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["first"]);
    assert!(iterator.base.finished);
}

#[tokio::test]
async fn chunk_processing_detects_the_legacy_completed_status() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[&format!("data: {COMPLETED_STATUS}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["int_1"]);
    assert_eq!(
        iterator
            .base
            .completed_response
            .as_ref()
            .and_then(|response| response.id.as_deref()),
        Some("int_1")
    );
    assert_eq!(logging.records().len(), 1);
}

#[tokio::test]
async fn chunk_processing_detects_the_new_completed_event_type() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[&format!("data: {COMPLETED_EVENT}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["int_2"]);
    assert_eq!(logging.records().len(), 1);
}

#[tokio::test]
async fn async_iteration_yields_events_in_order_and_finishes() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            r#"data: {"id":"a","event_type":"interaction.created"}"#,
            r#"data: {"id":"b","event_type":"step.delta","index":0}"#,
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["a", "b"]);
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().await.is_none());
    assert!(logging.records().is_empty());
}

#[tokio::test]
async fn async_iteration_dispatches_completion_with_prefer_async() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[&format!("data: {COMPLETED_STATUS}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    drain_async(&mut iterator).await;
    let records = logging.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].handler, "dispatch_success_handlers");
    assert_eq!(records[0].response.id.as_deref(), Some("int_1"));
    assert_eq!(records[0].start_time, moment());
    assert_eq!(records[0].end_time, ticked(1));
    assert_eq!(records[0].cache_hit, None);
    assert_eq!(records[0].prefer_async_handlers, Some(true));
}

#[tokio::test]
async fn async_iteration_skips_empty_results_between_events() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&["data: ", "", r#"data: {"id":"only"}"#, "data: [DONE]"]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["only"]);
}

#[test]
fn sync_iteration_yields_events_in_order_and_finishes() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&[
            r#"data: {"id":"a"}"#,
            r#"data: {"id":"b"}"#,
            "data: [DONE]",
            r#"data: {"id":"after"}"#,
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_sync(&mut iterator);
    assert_eq!(ids(&events), vec!["a", "b"]);
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().is_none());
}

#[test]
fn sync_iteration_skips_lines_without_events() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&["", "data: not json", r#"data: {"id":"only"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_sync(&mut iterator);
    assert_eq!(ids(&events), vec!["only"]);
    assert!(iterator.base.finished);
}

#[test]
fn sync_iteration_dispatches_the_async_handler_then_the_sync_handler() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&[&format!("data: {COMPLETED_EVENT}")]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    drain_sync(&mut iterator);
    let records = logging.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].handler, "async_success_handler");
    assert_eq!(records[1].handler, "success_handler");
    assert_eq!(records[0].response, records[1].response);
    assert_eq!(records[0].response.id.as_deref(), Some("int_2"));
    assert_eq!(records[0].start_time, moment());
    assert_eq!(records[1].start_time, moment());
    assert_eq!(records[0].end_time, ticked(1));
    assert_eq!(records[1].end_time, ticked(2));
    assert_eq!(records[0].cache_hit, None);
    assert_eq!(records[1].cache_hit, None);
    assert_eq!(records[0].prefer_async_handlers, None);
    assert_eq!(records[1].prefer_async_handlers, None);
}

async fn serve_stream(listener: TcpListener, answer: &str) {
    let (mut stream, _) = listener.accept().await.expect("the client connects");
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .await
            .expect("the request head arrives");
        head.push(byte[0]);
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nx-ratelimit-remaining-requests: 9\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    );
    stream
        .write_all(reply.as_bytes())
        .await
        .expect("the reply is written");
    stream.shutdown().await.expect("the connection closes");
}

#[tokio::test]
async fn async_iteration_reads_the_first_line_of_a_streamed_http_response() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the port binds");
    let address = listener.local_addr().expect("the address resolves");
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("the client builds");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&format!("http://{address}/stream")).expect("the url parses"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(30),
    };
    let answer = format!("data: {COMPLETED_STATUS}\n\ndata: [DONE]\n");
    let (response, _) = tokio::join!(client.send(request), serve_stream(listener, &answer));
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        response.expect("the stream response arrives"),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the http iterator builds");
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    let first = iterator
        .lines
        .next_line()
        .await
        .expect("the first line is read")
        .expect("the first line arrives");
    assert_eq!(first, format!("data: {COMPLETED_STATUS}"));
    assert!(!iterator.base.finished);
}

#[test]
fn sync_iteration_reads_a_buffered_response() {
    let response = InteractionBufferedResponse {
        headers: IndexMap::from([(
            String::from("x-ratelimit-remaining-requests"),
            String::from("9"),
        )]),
        body: format!("data: {COMPLETED_EVENT}\n\ndata: [DONE]\n").into_bytes(),
    };
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator =
        SyncInteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the buffered iterator builds");
    assert_eq!(
        iterator
            .base
            .hidden_params
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    let first = iterator
        .next_chunk()
        .expect("the first event arrives")
        .expect("the first event decodes");
    assert_eq!(first.id.as_deref(), Some("int_2"));
    assert!(iterator.next_chunk().is_none());
    assert_eq!(logging.records().len(), 2);
}

#[test]
fn buffered_response_yields_the_decoded_lines_before_the_line_error() {
    let mut response = InteractionBufferedResponse {
        headers: IndexMap::new(),
        body: format!("data: {COMPLETED_EVENT}\n\n").into_bytes(),
    };
    response.body.resize(
        response
            .body
            .len()
            .saturating_add(MAX_LINE_BYTES.saturating_add(1)),
        b'x',
    );
    let mut lines = response.iter_lines();
    assert_eq!(
        lines.next().transpose().expect("the first line decodes"),
        Some(format!("data: {COMPLETED_EVENT}"))
    );
    assert_eq!(
        lines.next().transpose().expect("the blank line decodes"),
        Some(String::new())
    );
    assert!(matches!(
        lines.next(),
        Some(Err(InteractionSourceError::Line(
            AlephAlphaError::LineTooLong { limit }
        ))) if limit == MAX_LINE_BYTES
    ));
    assert!(lines.next().is_none());
}

#[test]
fn sync_iteration_yields_earlier_events_before_the_line_error() {
    let mut body = format!("data: {COMPLETED_EVENT}\n\n").into_bytes();
    body.resize(
        body.len().saturating_add(MAX_LINE_BYTES).saturating_add(1),
        b'x',
    );
    let response = InteractionBufferedResponse {
        headers: IndexMap::new(),
        body,
    };
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator =
        SyncInteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the buffered iterator builds");
    let event = iterator
        .next_chunk()
        .expect("the event before the cut arrives")
        .expect("the event decodes");
    assert_eq!(event.id.as_deref(), Some("int_2"));
    assert!(!iterator.base.finished);
    let failure = iterator
        .next_chunk()
        .expect("the stream reports the line error");
    let Err(InteractionStreamError::Source {
        source: InteractionSourceError::Line(AlephAlphaError::LineTooLong { limit }),
    }) = failure
    else {
        panic!("the stream reports the over-limit line");
    };
    assert_eq!(limit, MAX_LINE_BYTES);
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().is_none());
    assert_eq!(logging.records().len(), 2);
}

#[tokio::test]
async fn error_handling_async_source_error_sets_finished_and_errors() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut response = FakeAsyncResponse::new(&[r#"data: {"id":"a"}"#]);
    response.lines.push(Err(FixtureSourceError));
    let mut iterator =
        InteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the fixture iterator builds");
    assert!(
        iterator
            .next_chunk()
            .await
            .expect("the first chunk")
            .is_ok()
    );
    let failure = iterator
        .next_chunk()
        .await
        .expect("the stream reports an error");
    assert!(matches!(
        failure,
        Err(InteractionStreamError::Source { .. })
    ));
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().await.is_none());
}

#[test]
fn error_handling_sync_source_error_sets_finished_and_errors() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut response = FakeSyncResponse::new(&[]);
    response.lines.push(Err(FixtureSourceError));
    let mut iterator =
        SyncInteractionsStreamIterator::new(response, setup(&mut logging, &config, &NO_API_BASE))
            .expect("the fixture iterator builds");
    let failure = iterator.next_chunk().expect("the stream reports an error");
    assert!(matches!(
        failure,
        Err(InteractionStreamError::Source { .. })
    ));
    assert!(iterator.base.finished);
    assert!(iterator.next_chunk().is_none());
}

#[tokio::test]
async fn error_handling_transform_error_is_returned_without_finishing() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[r#"data: {"id":5}"#, r#"data: {"id":"int_1"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let failure = iterator
        .next_chunk()
        .await
        .expect("the chunk is transformed");
    assert!(matches!(
        failure,
        Err(InteractionStreamError::Transform { .. })
    ));
    assert!(!iterator.base.finished);
    let next = iterator.next_chunk().await.expect("the stream continues");
    assert_eq!(
        next.expect("the second chunk decodes").id.as_deref(),
        Some("int_1")
    );
}

#[test]
fn error_handling_sync_transform_error_is_returned_without_finishing() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = SyncInteractionsStreamIterator::new(
        FakeSyncResponse::new(&[r#"data: {"id":5}"#, r#"data: {"id":"int_1"}"#]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let failure = iterator.next_chunk().expect("the chunk is transformed");
    assert!(matches!(
        failure,
        Err(InteractionStreamError::Transform { .. })
    ));
    assert!(!iterator.base.finished);
    let next = iterator.next_chunk().expect("the stream continues");
    assert_eq!(
        next.expect("the second chunk decodes").id.as_deref(),
        Some("int_1")
    );
}

#[tokio::test]
async fn multiple_completions_keep_the_last_response() {
    let config = FixtureConfig;
    let mut logging = RecordingLogging::new(Params::new());
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&[
            &format!("data: {COMPLETED_STATUS}"),
            r#"data: {"id":"int_3","event_type":"step.delta"}"#,
            &format!("data: {COMPLETED_EVENT}"),
        ]),
        setup(&mut logging, &config, &NO_API_BASE),
    )
    .expect("the fixture iterator builds");
    let events = drain_async(&mut iterator).await;
    assert_eq!(ids(&events), vec!["int_1", "int_3", "int_2"]);
    assert_eq!(
        iterator
            .base
            .completed_response
            .as_ref()
            .and_then(|response| response.id.as_deref()),
        Some("int_2")
    );
    let records = logging.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].response.id.as_deref(), Some("int_1"));
    assert_eq!(records[1].response.id.as_deref(), Some("int_2"));
}

fn alias_params(base_url: &str) -> Params {
    let fields = IndexMap::from([(
        String::from("base_url"),
        JsonValue::Text(base_url.to_owned()),
    )]);
    Params::from([(String::from("litellm_params"), JsonValue::Object(fields))])
}

include!("tests/support.rs");
include!("tests/corpus.rs");
