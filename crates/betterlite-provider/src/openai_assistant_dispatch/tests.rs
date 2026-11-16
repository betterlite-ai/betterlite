#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_config::tls::Verification;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::openai::{ChoiceValue, DecisionAnswer, DecisionRequestBody};
use betterlite_openai::assistants::client::{ClientKind, ClientOptions};
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::{AssistantCreate, ListParams};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::decisions::transformation::wire::ProviderRequestWire;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::error::RealtimeError;
use betterlite_openai::realtime::failure::{ClientAction, RealtimeClose};
use betterlite_openai::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use betterlite_openai::realtime::ssl::RealtimeSsl;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::operations::RunDispatch;
use betterlite_openai::thread_runs::records::RunRequest;
use betterlite_openai::thread_runs::stream::{RunChunks, RunStreamTransport};
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::operations::ThreadReply;
use betterlite_openai::threads::records::{MessageStatus, ThreadCreate, ThreadMessageData};
use futures_util::StreamExt as _;
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::operation::{OperationRequest, OperationResult};

use super::call::{
    AssistantsCall, AssistantsFacade, AssistantsOperation, DecisionsCall, DecisionsOperation,
    OpenAiAssistantOperation, RealtimeCall, RealtimeFailureOwned, RunsCall, ThreadsCall,
    ThreadsOperation,
};
use super::error::OpenAiAssistantError;
use super::execute::dispatch;
use super::reply::{AssistantsReply, DecisionsProjection, DecisionsReply, RunOutcome};
use super::{OpenAiAssistantRequest, OpenAiAssistantResult};

const BASE: &str = "https://example.test/v1";
const KEY: &str = "sk-fixture";
const ASSISTANT: &str =
    r#"{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}"#;
const PAGE: &str = r#"{"object":"list","data":[{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}],"first_id":"asst_1","last_id":"asst_1","has_more":true}"#;
const DELETED: &str = r#"{"id":"asst_1","deleted":true,"object":"assistant.deleted"}"#;
const THREAD: &str =
    r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"core"}}"#;
const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
const MESSAGE_PAGE: &str = r#"{"object":"list","data":[{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}],"first_id":"msg_1","last_id":"msg_1","has_more":false}"#;
const RUN: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_1","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}"#;
const ERROR_BODY: &str = r#"{"error":{"message":"nope","type":"invalid_request_error","param":"model","code":"model_not_found"}}"#;
const CHOICE_QUESTION: &str = r#"{"input":"Check this","questions":[{"type":"choice","name":"label","instructions":"Pick","choices":[{"value":"yes"},{"value":"no"}]}],"safety_identifier":"safe-1"}"#;
const CHOICE_WIRE: &str = r#"{"model":"gpt-4o","input":"Check this","questions":[{"type":"choice","name":"label","instructions":"Pick","choices":[{"value":"yes"},{"value":"no"}]}],"safety_identifier":"safe-1"}"#;
const CHOICE_ANSWERS: &str = r#"{"model":"gpt-4o","answers":[{"type":"choice","name":"label","choice":"yes","probabilities":[{"value":"yes","probability":0.9},{"value":"no","probability":0.1}],"confidence":0.9}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#;
const RUN_BODY: &str = r#"{"assistant_id":"asst_1","additional_instructions":null,"instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":false,"tools":[{"type":"code_interpreter"}]}"#;
const STREAM_BODY: &str = r#"{"assistant_id":"asst_1","additional_instructions":null,"instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":true,"tools":[{"type":"code_interpreter"}]}"#;

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    url: String,
    authorization: Option<String>,
    organization: Option<String>,
    beta: Option<String>,
    content_type: Option<String>,
    body: Option<JsonValue>,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, String)>>,
    stream: Mutex<Option<String>>,
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?;
    Some(value.to_str().unwrap().to_owned())
}

impl Recorder {
    fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back((status, body.to_owned()));
    }

    fn queue_stream(&self, body: &str) {
        *self.stream.lock().unwrap() = Some(body.to_owned());
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn record(&self, request: &OutboundRequest) {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body: Option<JsonValue> = match &request.body {
            RequestBody::Json(bytes) => Some(serde_json::from_slice(bytes).unwrap()),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
        };
        let mut content_type = header(&request.headers, "content-type");
        if content_type.is_none() && body.is_some() {
            content_type = Some(String::from("application/json"));
        }
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: url.as_str().to_owned(),
            authorization: header(&request.headers, "authorization"),
            organization: header(&request.headers, "openai-organization"),
            beta: header(&request.headers, "openai-beta"),
            content_type,
            body,
        });
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        let (status, body) = self.replies.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

impl RunStreamTransport for Recorder {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        self.record(&request);
        let body = self.stream.lock().unwrap().take().unwrap_or_default();
        let chunks: RunChunks = Box::pin(futures_util::stream::once(async move {
            Ok::<Vec<u8>, ThreadRunsError>(body.into_bytes())
        }));
        ready(Ok(chunks))
    }
}

#[derive(Debug)]
struct RecordedCall {
    api_base: String,
    headers: Headers,
    query_params: QueryParams,
}

#[derive(Default)]
struct RecordingLogger {
    calls: Vec<RecordedCall>,
}

impl RealtimeLogger for RecordingLogger {
    fn pre_call(&mut self, record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        self.calls.push(RecordedCall {
            api_base: record.api_base.to_owned(),
            headers: record.headers.clone(),
            query_params: record.query_params.clone(),
        });
        Ok(())
    }
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn options() -> ClientOptions {
    ClientOptions {
        api_key: Some(SecretString::from(KEY)),
        api_base: Some(String::from(BASE)),
        timeout: None,
        max_retries: None,
        organization: Some(String::from("org-fixture")),
    }
}

async fn run_operation(
    transport: &Arc<Recorder>,
    logger: &mut RecordingLogger,
    operation: OpenAiAssistantOperation,
) -> Result<OpenAiAssistantResult, OpenAiAssistantError> {
    let result = dispatch(
        OperationRequest::OpenAiAssistant(OpenAiAssistantRequest),
        operation,
        Arc::clone(transport),
        logger,
    )
    .await?;
    let OperationResult::OpenAiAssistant(result) = result else {
        panic!("the assistant composition returned another operation")
    };
    Ok(result)
}

fn assistant_create() -> AssistantCreate {
    serde_json::from_str(r#"{"model":"gpt-4o","name":"helper"}"#).unwrap()
}

fn assistant_body() -> JsonValue {
    json(r#"{"model":"gpt-4o","name":"helper"}"#)
}

fn assistants_operation(operation: AssistantsOperation) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Assistants(AssistantsCall {
        options: options(),
        operation,
    })
}

fn caller_params() -> ListParams {
    ListParams {
        order: Some(String::from("asc")),
        limit: Some(5),
        before: Some(String::from("b")),
        after: Some(String::from("a")),
    }
}

async fn assistants_create() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, ASSISTANT);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::Create {
        data: assistant_create(),
        async_requested: true,
    });
    let created = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Assistants(AssistantsReply::Assistant(assistant)) = created else {
        panic!("create did not return an assistant")
    };
    assert_eq!(assistant.id, "asst_1");
    assert_eq!(assistant.model, "gpt-4o");
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/assistants"));
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-fixture"));
    assert_eq!(seen[0].organization.as_deref(), Some("org-fixture"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    assert_eq!(seen[0].body.as_ref(), Some(&assistant_body()));
}

async fn assistants_delete() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, DELETED);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::Delete {
        assistant_id: String::from("a/b c"),
        async_requested: false,
    });
    let deleted = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Assistants(AssistantsReply::Deleted(deleted)) = deleted else {
        panic!("delete did not return a deletion")
    };
    assert!(deleted.deleted);
    assert_eq!(deleted.id, "asst_1");
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "DELETE");
    assert_eq!(seen[0].url, format!("{BASE}/assistants/a%2Fb%20c"));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    assert_eq!(seen[0].body, None);
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

async fn assistants_listing() {
    let transport = Arc::new(Recorder::default());
    for _ in 0..3 {
        transport.queue(200, PAGE);
    }
    let mut logger = RecordingLogger::default();
    let facade = assistants_operation(AssistantsOperation::List {
        facade: AssistantsFacade::Facade {
            aget_assistants: true,
        },
        params: caller_params(),
    });
    let page = run_operation(&transport, &mut logger, facade)
        .await
        .unwrap();
    let OpenAiAssistantResult::Assistants(AssistantsReply::Page(page)) = page else {
        panic!("the facade listing did not return a page")
    };
    assert_eq!(page.data[0].id, "asst_1");
    assert_eq!(page.has_more, Some(true));
    let sync = assistants_operation(AssistantsOperation::List {
        facade: AssistantsFacade::Facade {
            aget_assistants: false,
        },
        params: caller_params(),
    });
    run_operation(&transport, &mut logger, sync).await.unwrap();
    let direct = assistants_operation(AssistantsOperation::List {
        facade: AssistantsFacade::AsyncHelper,
        params: caller_params(),
    });
    run_operation(&transport, &mut logger, direct)
        .await
        .unwrap();
    let seen = transport.requests();
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen[0].url,
        format!("{BASE}/assistants?limit=20&order=desc")
    );
    assert_eq!(
        seen[1].url,
        format!("{BASE}/assistants?after=a&before=b&limit=5&order=asc")
    );
    assert_eq!(
        seen[2].url,
        format!("{BASE}/assistants?after=a&before=b&limit=5&order=asc")
    );
    for request in &seen {
        assert_eq!(request.beta.as_deref(), Some("assistants=v2"));
        assert_eq!(request.authorization.as_deref(), Some("Bearer sk-fixture"));
    }
}

async fn assistants_leaf_error() {
    let transport = Arc::new(Recorder::default());
    transport.queue(404, ERROR_BODY);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::Delete {
        assistant_id: String::from("asst_missing"),
        async_requested: true,
    });
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the missing assistant error did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Assistants(AssistantsError::Status { status: 404, .. })
    ));
}

async fn assistants_case() {
    assistants_create().await;
    assistants_delete().await;
    assistants_listing().await;
    assistants_leaf_error().await;
}

fn thread_data() -> ThreadCreate {
    serde_json::from_str(
        r#"{"metadata":{"team":"core"},"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .unwrap()
}

fn message_data() -> ThreadMessageData {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap()
}

fn threads_operation(operation: ThreadsOperation) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Threads(ThreadsCall {
        options: options(),
        operation,
    })
}

async fn threads_create() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, THREAD);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::Create {
        data: thread_data(),
        async_requested: true,
    });
    let created = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(ThreadReply::Thread(thread)) = created else {
        panic!("create did not return a thread")
    };
    assert_eq!(thread.id, "thread_1");
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
    assert_eq!(
        seen[0].body.as_ref(),
        Some(&json(
            r#"{"metadata":{"team":"core"},"messages":[{"role":"user","content":"hi"}]}"#
        ))
    );
}

async fn threads_retrieve() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, THREAD);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::Retrieve {
        thread_id: String::from("thread_1"),
        async_requested: false,
    });
    let retrieved = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(ThreadReply::Thread(thread)) = retrieved else {
        panic!("retrieve did not return a thread")
    };
    assert_eq!(thread.id, "thread_1");
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1"));
    assert_eq!(seen[0].body, None);
}

async fn threads_add_message() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, MESSAGE);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::AddMessage {
        thread_id: String::from("thread_1"),
        data: message_data(),
        async_requested: true,
    });
    let added = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(ThreadReply::Message(message)) = added else {
        panic!("add_message did not return a message")
    };
    assert_eq!(message.id, "msg_1");
    assert_eq!(message.status, Some(MessageStatus::Completed));
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1/messages"));
    assert_eq!(
        seen[0].body.as_ref(),
        Some(&json(r#"{"role":"user","content":"hi"}"#))
    );
}

async fn threads_list_messages() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, MESSAGE_PAGE);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::ListMessages {
        thread_id: String::from("thread_1"),
        async_requested: false,
    });
    let listed = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(ThreadReply::Page(page)) = listed else {
        panic!("list_messages did not return a page")
    };
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, "msg_1");
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1/messages"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

async fn threads_missing_key() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = OpenAiAssistantOperation::Threads(ThreadsCall {
        options: ClientOptions::default(),
        operation: ThreadsOperation::Retrieve {
            thread_id: String::from("thread_1"),
            async_requested: false,
        },
    });
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the missing api key did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Threads(ThreadsError::MissingApiKey)
    ));
}

async fn threads_case() {
    threads_create().await;
    threads_retrieve().await;
    threads_add_message().await;
    threads_list_messages().await;
    threads_missing_key().await;
}

fn run_request() -> RunRequest {
    serde_json::from_str(r#"{"thread_id":"thread_1","assistant_id":"asst_1","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","tools":[{"type":"code_interpreter"}]}"#).unwrap()
}

fn runs_operation(dispatch: RunDispatch) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Runs(RunsCall {
        options: options(),
        dispatch,
        request: run_request(),
    })
}

fn stream_body() -> String {
    format!(
        "event: thread.run.created\ndata: {RUN}\n\nevent: thread.run.completed\ndata: {RUN}\n\ndata: [DONE]\n\n"
    )
}

async fn runs_buffered() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, RUN);
    transport.queue(200, RUN);
    let mut logger = RecordingLogger::default();
    let outcome = run_operation(
        &transport,
        &mut logger,
        runs_operation(RunDispatch::default()),
    )
    .await
    .unwrap();
    let OpenAiAssistantResult::Runs(RunOutcome::Polled(kind, reply)) = outcome else {
        panic!("a buffered run did not return a polled reply")
    };
    assert_eq!(kind, ClientKind::Sync);
    assert_eq!(reply.id, "run_1");
    assert_eq!(reply.status, "completed");
    let seen = transport.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1/runs"));
    assert_eq!(seen[0].body.as_ref(), Some(&json(RUN_BODY)));
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].url, format!("{BASE}/threads/thread_1/runs/run_1"));
}

async fn runs_async_facade() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, RUN);
    transport.queue(200, RUN);
    let mut logger = RecordingLogger::default();
    let dispatch = RunDispatch {
        arun_thread: true,
        stream: false,
    };
    let outcome = run_operation(&transport, &mut logger, runs_operation(dispatch))
        .await
        .unwrap();
    let OpenAiAssistantResult::Runs(RunOutcome::Polled(kind, reply)) = outcome else {
        panic!("the async facade did not return a polled reply")
    };
    assert_eq!(kind, ClientKind::Async);
    assert_eq!(reply.status, "completed");
    let seen = transport.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body.as_ref(), Some(&json(RUN_BODY)));
}

async fn runs_streaming() {
    let transport = Arc::new(Recorder::default());
    transport.queue_stream(&stream_body());
    let mut logger = RecordingLogger::default();
    let dispatch = RunDispatch {
        arun_thread: false,
        stream: true,
    };
    let outcome = run_operation(&transport, &mut logger, runs_operation(dispatch))
        .await
        .unwrap();
    let OpenAiAssistantResult::Runs(RunOutcome::Streaming(mut events)) = outcome else {
        panic!("a streaming run did not stay a stream")
    };
    let mut names: Vec<String> = Vec::new();
    while let Some(event) = events.next().await {
        names.push(event.unwrap().name().to_owned());
    }
    assert_eq!(names, ["thread.run.created", "thread.run.completed"]);
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].body.as_ref(), Some(&json(STREAM_BODY)));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

async fn runs_leaf_error() {
    let transport = Arc::new(Recorder::default());
    transport.queue(500, ERROR_BODY);
    let mut logger = RecordingLogger::default();
    let call = runs_operation(RunDispatch::default());
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the run status error did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Runs(ThreadRunsError::Status(500, _))
    ));
}

async fn runs_case() {
    runs_buffered().await;
    runs_async_facade().await;
    runs_streaming().await;
    runs_leaf_error().await;
}

fn decision_request() -> DecisionRequestBody {
    serde_json::from_str(CHOICE_QUESTION).unwrap()
}

fn decisions_call(
    api_base: Option<&str>,
    api_key: Option<&str>,
    operation: DecisionsOperation,
) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Decisions(DecisionsCall {
        api_base: api_base.map(String::from),
        api_key: api_key.map(SecretString::from),
        operation,
    })
}

async fn decisions_request() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(
        Some(BASE),
        Some("sk-decisions"),
        DecisionsOperation::Request {
            model: String::from("gpt-4o"),
            request: decision_request(),
        },
    );
    let projected = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Decisions(DecisionsReply {
        api_base,
        api_key,
        projection,
    }) = projected
    else {
        panic!("the decisions request did not project")
    };
    assert_eq!(api_base.as_deref(), Some(BASE));
    assert_eq!(
        api_key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("sk-decisions")
    );
    let DecisionsProjection::Request(wire) = projection else {
        panic!("the request call produced a response projection")
    };
    assert!(matches!(&wire, ProviderRequestWire::OpenAi(_)));
    let written: JsonValue = serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(written, json(CHOICE_WIRE));
    assert!(transport.requests().is_empty());
}

async fn decisions_defaults() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(
        None,
        None,
        DecisionsOperation::Request {
            model: String::from("gpt-4o"),
            request: decision_request(),
        },
    );
    let projected = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Decisions(reply) = projected else {
        panic!("the default resolution did not project")
    };
    assert_eq!(reply.api_base.as_deref(), Some("https://api.openai.com"));
    assert!(reply.api_key.is_none());
}

async fn decisions_response() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(
        Some(BASE),
        Some("sk-decisions"),
        DecisionsOperation::Response {
            model: String::from("gpt-4o"),
            request: decision_request(),
            payload: CHOICE_ANSWERS.as_bytes().to_vec(),
        },
    );
    let projected = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Decisions(DecisionsReply {
        projection: DecisionsProjection::Response(response),
        ..
    }) = projected
    else {
        panic!("the response call did not project its answer")
    };
    assert_eq!(response.model, "gpt-4o");
    assert_eq!(response.usage.total_tokens.0.as_u64(), Some(5));
    let DecisionAnswer::Choice { name, choice, .. } = &response.answers[0] else {
        panic!("the projected answer did not stay a choice")
    };
    assert_eq!(name.as_deref(), Some("label"));
    assert!(matches!(choice, ChoiceValue::Text(text) if text == "yes"));
}

async fn decisions_malformed_payload() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(
        Some(BASE),
        Some("sk-decisions"),
        DecisionsOperation::Response {
            model: String::from("gpt-4o"),
            request: decision_request(),
            payload: b"{not json".to_vec(),
        },
    );
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the malformed decisions payload did not surface")
    };
    assert!(matches!(error, OpenAiAssistantError::DecisionsPayload(_)));
}

async fn decisions_case() {
    decisions_request().await;
    decisions_defaults().await;
    decisions_response().await;
    decisions_malformed_payload().await;
}

fn realtime_params() -> QueryParams {
    QueryParams::from([
        (String::from("model"), Some(String::from("realtime-a"))),
        (String::from("intent"), Some(String::from("transcription"))),
    ])
}

fn realtime_connect(
    api_base: &str,
    beta_realtime: bool,
    ssl_verification: Verification,
) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: Some(String::from(api_base)),
        api_key: Some(SecretString::from(KEY)),
        model: String::from("realtime-a"),
        query_params: Some(realtime_params()),
        beta_realtime,
        ssl_verification,
        max_message_size_bytes: 4096,
    })
}

fn realtime_failure(failure: RealtimeFailureOwned) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Realtime(RealtimeCall::Failure(failure))
}

async fn realtime_secure_session() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = realtime_connect("https://example.test/", true, Verification::Enabled);
    let prepared = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Session(plan) = prepared else {
        panic!("the session preparation did not return a session")
    };
    assert_eq!(
        plan.url,
        "wss://example.test/v1/realtime?model=realtime-a&intent=transcription"
    );
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-fixture")
    );
    assert_eq!(
        plan.headers.get("OpenAI-Beta").map(String::as_str),
        Some("realtime=v1")
    );
    assert_eq!(plan.ssl, RealtimeSsl::Shared);
    assert_eq!(plan.max_message_size_bytes, 4096);
    assert_eq!(
        plan.force_transcription_model.as_deref(),
        Some("realtime-a")
    );
    assert!(!plan.event_normalizer_installed);
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(logger.calls[0].api_base, plan.url);
    assert_eq!(logger.calls[0].query_params, realtime_params());
    assert!(transport.requests().is_empty());
}

async fn realtime_plain_session() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: Some(String::from("http://127.0.0.1:8080/base")),
        api_key: Some(SecretString::from(KEY)),
        model: String::from("realtime-a"),
        query_params: None,
        beta_realtime: false,
        ssl_verification: Verification::Enabled,
        max_message_size_bytes: 1024,
    });
    let prepared = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Session(plan) = prepared else {
        panic!("the plain session preparation did not return a session")
    };
    assert_eq!(plan.url, "ws://127.0.0.1:8080/v1/realtime?model=realtime-a");
    assert_eq!(plan.ssl, RealtimeSsl::None);
    assert_eq!(plan.force_transcription_model, None);
    assert_eq!(plan.headers.get("OpenAI-Beta"), None);
    assert_eq!(plan.max_message_size_bytes, 1024);
}

async fn realtime_insecure_session() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = realtime_connect(BASE, false, Verification::Disabled);
    let prepared = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Session(plan) = prepared else {
        panic!("the insecure session preparation did not return a session")
    };
    assert!(plan.url.starts_with("wss://"));
    assert_eq!(plan.ssl, RealtimeSsl::Insecure);
    assert_eq!(plan.headers.get("OpenAI-Beta"), None);
}

async fn realtime_missing_key() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: None,
        api_key: None,
        model: String::from("realtime-a"),
        query_params: None,
        beta_realtime: false,
        ssl_verification: Verification::Enabled,
        max_message_size_bytes: 1024,
    });
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the missing realtime api key did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Realtime(RealtimeError::MissingApiKey)
    ));
}

async fn realtime_connection_failures() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let handshake = realtime_failure(RealtimeFailureOwned::Handshake { status_code: 403 });
    let action = run_operation(&transport, &mut logger, handshake)
        .await
        .unwrap();
    let OpenAiAssistantResult::SessionFailure(action) = action else {
        panic!("a handshake refusal did not return a session failure")
    };
    assert_eq!(action, ClientAction::RefuseHandshake { status_code: 403 });
    let other = realtime_failure(RealtimeFailureOwned::Other {
        message: String::from("connection reset"),
    });
    let action = run_operation(&transport, &mut logger, other).await.unwrap();
    let OpenAiAssistantResult::SessionFailure(action) = action else {
        panic!("a connection failure did not return a session failure")
    };
    assert_eq!(
        action,
        ClientAction::Close(RealtimeClose {
            code: 1011,
            reason: String::from("Internal server error: connection reset"),
        })
    );
}

async fn realtime_close_failures() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let ignored = realtime_failure(RealtimeFailureOwned::CloseFailed {
        message: String::from("connection reset"),
        close_error: String::from("already completed"),
    });
    let action = run_operation(&transport, &mut logger, ignored)
        .await
        .unwrap();
    let OpenAiAssistantResult::SessionFailure(action) = action else {
        panic!("an ignored close failure did not return a session failure")
    };
    assert!(matches!(action, ClientAction::IgnoreCloseFailure { .. }));
    let unexpected = realtime_failure(RealtimeFailureOwned::CloseFailed {
        message: String::from("connection reset"),
        close_error: String::from("socket gone"),
    });
    let Err(error) = run_operation(&transport, &mut logger, unexpected).await else {
        panic!("an unexpected close failure did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Realtime(RealtimeError::UnexpectedCloseFailure { .. })
    ));
}

async fn realtime_session_case() {
    realtime_secure_session().await;
    realtime_plain_session().await;
    realtime_insecure_session().await;
    realtime_missing_key().await;
    realtime_connection_failures().await;
    realtime_close_failures().await;
}

#[derive(Debug, Clone, Copy)]
enum Scenario {
    Assistants,
    Threads,
    Runs,
    Decisions,
    RealtimeSession,
}

#[rstest]
#[case::assistants(Scenario::Assistants)]
#[case::threads(Scenario::Threads)]
#[case::runs(Scenario::Runs)]
#[case::decisions(Scenario::Decisions)]
#[case::realtime_session(Scenario::RealtimeSession)]
#[tokio::test]
async fn openai_assistant_dispatch_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Assistants => assistants_case().await,
        Scenario::Threads => threads_case().await,
        Scenario::Runs => runs_case().await,
        Scenario::Decisions => decisions_case().await,
        Scenario::RealtimeSession => realtime_session_case().await,
    }
}

#[tokio::test]
async fn openai_assistant_dispatch_unsupported_operation() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let operation = assistants_operation(AssistantsOperation::Delete {
        assistant_id: String::from("asst_1"),
        async_requested: true,
    });
    let Err(error) = dispatch(
        OperationRequest::OpenAi(crate::openai_dispatch::OpenAiRequest),
        operation,
        transport,
        &mut logger,
    )
    .await
    else {
        panic!("another operation reached the assistant composition")
    };
    assert!(matches!(error, OpenAiAssistantError::UnsupportedOperation));
}
