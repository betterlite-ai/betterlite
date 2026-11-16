#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use betterlite_config::tls::Verification;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::openai::{DecisionRequestBody, DecisionResponse};
use betterlite_openai::assistants::client::{ClientKind, ClientOptions};
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::{AssistantCreate, ListParams};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::decisions::openai::response::to_openai::ir_to_openai_response;
use betterlite_openai::decisions::openai::request::openai_request_to_ir;
use betterlite_openai::decisions::transformation::requests::RequestConversionError;
use betterlite_openai::decisions::transformation::wire::ProviderRequestWire;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::failure::{ClientAction, RealtimeClose};
use betterlite_openai::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use betterlite_openai::realtime::ssl::RealtimeSsl;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::operations::RunDispatch;
use betterlite_openai::thread_runs::records::{RunRequest, RunReply};
use betterlite_openai::thread_runs::stream::{RunChunks, RunStreamTransport};
use betterlite_openai::threads::operations::ThreadReply;
use betterlite_openai::threads::records::{MessageStatus, ThreadCreate, ThreadMessageData};
use futures_util::StreamExt as _;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::operation::{OperationRequest, OperationResult};

use super::call::{
    AssistantsCall, AssistantsFacade, AssistantsOperation, DecisionsCall, DecisionsOperation,
    RealtimeCall, RealtimeFailureOwned, RunsCall, ThreadsCall, ThreadsOperation,
};
use super::error::OpenAiAssistantError;
use super::execute::dispatch;
use super::reply::{AssistantsReply, DecisionsProjection, DecisionsReply, RunOutcome};
use super::{
    OpenAiAssistantRequest, OpenAiAssistantResult, OpenAiAssistantPrepared, SessionNormalizerKind,
};

const BASE: &str = "https://example.test/v1";
const KEY: &str = "sk-fixture";
const ASSISTANT: &str = r#"{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}"#;
const PAGE: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
const DELETED: &str = r#"{"id":"asst_1","deleted":true,"object":"assistant.deleted"}"#;
const THREAD: &str = r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"core"}}"#;
const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
const MESSAGE_PAGE: &str = r#"{"object":"list","data":[{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}],"first_id":"msg_1","last_id":"msg_1","has_more":false}"#;
const RUN: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_1","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}"#;
const ERROR_BODY: &str = r#"{"error":{"message":"nope","type":"invalid_request_error"}}"#;
const CHOICE_QUESTION: &str = r#"{"input":"Check this","questions":[{"type":"choice","name":"label","instructions":"Pick","choices":[{"value":"yes"},{"value":"no"}]}],"safety_identifier":"safe-1"}"#;

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    method: String,
    url: String,
    authorization: Option<String>,
    organization: Option<String>,
    beta: Option<String>,
    content_type: Option<String>,
    body: Option<Value>,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, String)>>,
    chunks: Mutex<Option<String>>,
    stream_status: Mutex<Option<(u16, Vec<u8>)>>,
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
        *self.chunks.lock().unwrap() = Some(body.to_owned());
    }

    fn fail_stream(&self, status: u16, body: &str) {
        *self.stream_status.lock().unwrap() = Some((status, body.as_bytes().to_vec()));
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn record(&self, request: &OutboundRequest) {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = match &request.body {
            RequestBody::Json(bytes) => Some(serde_json::from_slice(bytes).unwrap()),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
        };
        let content_type = header(&request.headers, "content-type")
            .or_else(|| body.is_some().then(|| String::from("application/json")));
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
        let failure = self.stream_status.lock().unwrap().take();
        let body = self.chunks.lock().unwrap().take().unwrap_or_default();
        ready(match failure {
            Some((status, body)) => Err(ThreadRunsError::Status(status, body)),
            None => {
                let chunks: RunChunks = Box::pin(futures_util::stream::once(async move {
                    Ok::<Vec<u8>, ThreadRunsError>(body.into_bytes())
                }));
                Ok(chunks)
            }
        })
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

fn options(key: &str) -> ClientOptions {
    ClientOptions {
        api_key: Some(SecretString::from(key)),
        api_base: Some(String::from(BASE)),
        timeout: None,
        max_retries: None,
        organization: Some(String::from("org-fixture")),
    }
}

fn run_request() -> RunRequest {
    serde_json::from_value(json!({
        "thread_id": "thread_1",
        "assistant_id": "asst_1",
        "instructions": "base",
        "metadata": {"tag": "x"},
        "model": "gpt-4o-mini",
        "tools": [{"type": "code_interpreter"}],
    }))
    .unwrap()
}

fn stream_body() -> String {
    format!(
        "event: thread.run.created\ndata: {RUN}\n\nevent: thread.run.completed\ndata: {RUN}\n\ndata: [DONE]\n\n"
    )
}

async fn assistants_case() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, ASSISTANT);
    transport.queue(200, PAGE);
    transport.queue(200, PAGE);
    transport.queue(200, DELETED);
    transport.queue(404, ERROR_BODY);
    let mut logger = RecordingLogger::default();
    let data: AssistantCreate =
        serde_json::from_value(json!({"model": "gpt-4o", "name": "helper"})).unwrap();
    let create = OpenAiAssistantRequest::Assistants(AssistantsCall {
        options: options(KEY),
        operation: AssistantsOperation::Create {
            data,
            async_requested: true,
        },
    });
    let created = dispatch(
        OperationRequest::OpenAiAssistant(create),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Assistants(
        AssistantsReply::Assistant(assistant),
    )) = created
    else {
        panic!("create did not return an assistant")
    };
    assert_eq!(assistant.id, "asst_1");

    let list = OpenAiAssistantRequest::Assistants(AssistantsCall {
        options: options(KEY),
        operation: AssistantsOperation::List {
            facade: AssistantsFacade::Facade {
                aget_assistants: true,
            },
            params: ListParams::default(),
        },
    });
    dispatch(
        OperationRequest::OpenAiAssistant(list),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();

    let direct = OpenAiAssistantRequest::Assistants(AssistantsCall {
        options: options(KEY),
        operation: AssistantsOperation::List {
            facade: AssistantsFacade::AsyncHelper,
            params: ListParams {
                order: Some(String::from("asc")),
                limit: Some(5),
                before: Some(String::from("b")),
                after: Some(String::from("a")),
            },
        },
    });
    dispatch(
        OperationRequest::OpenAiAssistant(direct),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();

    let delete = OpenAiAssistantRequest::Assistants(AssistantsCall {
        options: options(KEY),
        operation: AssistantsOperation::Delete {
            assistant_id: String::from("a/b c"),
            async_requested: false,
        },
    });
    let deleted = dispatch(
        OperationRequest::OpenAiAssistant(delete),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Assistants(
        AssistantsReply::Deleted(deleted),
    )) = deleted
    else {
        panic!("delete did not return a deletion")
    };
    assert!(deleted.deleted);

    let seen = transport.requests();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/assistants"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({"model": "gpt-4o", "name": "helper"})
    );
    assert_eq!(
        seen[1].url,
        format!("{BASE}/assistants?limit=20&order=desc")
    );
    assert_eq!(
        seen[1].beta.as_deref(),
        Some("assistants=v2"),
        "the facade listing keeps the assistants beta header"
    );
    assert_eq!(
        seen[2].url,
        format!("{BASE}/assistants?after=a&before=b&limit=5&order=asc")
    );
    assert_eq!(seen[3].url, format!("{BASE}/assistants/a%2Fb%20c"));
    assert_eq!(seen[3].method, "DELETE");
    assert_eq!(seen[3].content_type.as_deref(), Some("application/json"));

    let failure = OpenAiAssistantRequest::Assistants(AssistantsCall {
        options: options(KEY),
        operation: AssistantsOperation::Delete {
            assistant_id: String::from("asst_missing"),
            async_requested: true,
        },
    });
    let error = dispatch(
        OperationRequest::OpenAiAssistant(failure),
        transport,
        &mut logger,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        OpenAiAssistantError::Assistants(AssistantsError::Status { status: 404, .. })
    ));
}

async fn threads_case() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, THREAD);
    transport.queue(200, THREAD);
    transport.queue(200, MESSAGE);
    transport.queue(200, MESSAGE_PAGE);
    let mut logger = RecordingLogger::default();
    let data: ThreadCreate = serde_json::from_value(json!({
        "metadata": {"team": "core"},
        "messages": [{"role": "user", "content": "hi"}],
    }))
    .unwrap();
    let create = OpenAiAssistantRequest::Threads(ThreadsCall {
        options: options(KEY),
        operation: ThreadsOperation::Create {
            data,
            async_requested: true,
        },
    });
    let thread = dispatch(
        OperationRequest::OpenAiAssistant(create),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Threads(ThreadReply::Thread(
        thread,
    ))) = thread
    else {
        panic!("create did not return a thread")
    };
    assert_eq!(thread.id, "thread_1");

    let retrieve = OpenAiAssistantRequest::Threads(ThreadsCall {
        options: options(KEY),
        operation: ThreadsOperation::Retrieve {
            thread_id: String::from("thread_1"),
            async_requested: false,
        },
    });
    dispatch(
        OperationRequest::OpenAiAssistant(retrieve),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();

    let message: ThreadMessageData =
        serde_json::from_value(json!({"role": "user", "content": "hi"})).unwrap();
    let add = OpenAiAssistantRequest::Threads(ThreadsCall {
        options: options(KEY),
        operation: ThreadsOperation::AddMessage {
            thread_id: String::from("thread_1"),
            data: message,
            async_requested: false,
        },
    });
    let added = dispatch(
        OperationRequest::OpenAiAssistant(add),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Threads(ThreadReply::Message(
        message,
    ))) = added
    else {
        panic!("add_message did not return a message")
    };
    assert_eq!(message.status, Some(MessageStatus::Completed));

    let list = OpenAiAssistantRequest::Threads(ThreadsCall {
        options: options(KEY),
        operation: ThreadsOperation::ListMessages {
            thread_id: String::from("thread_1"),
            async_requested: true,
        },
    });
    let listed = dispatch(
        OperationRequest::OpenAiAssistant(list),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Threads(ThreadReply::Page(page))) =
        listed
    else {
        panic!("list_messages did not return a page")
    };
    assert_eq!(page.data.len(), 1);

    let seen = transport.requests();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({"metadata": {"team": "core"}, "messages": [{"role": "user", "content": "hi"}]})
    );
    assert_eq!(seen[1].url, format!("{BASE}/threads/thread_1"));
    assert_eq!(seen[2].method, "POST");
    assert_eq!(seen[2].url, format!("{BASE}/threads/thread_1/messages"));
    assert_eq!(
        seen[2].body.as_ref().unwrap(),
        &json!({"role": "user", "content": "hi"})
    );
    assert_eq!(seen[3].url, format!("{BASE}/threads/thread_1/messages"));
    for request in &seen {
        assert_eq!(request.beta.as_deref(), Some("assistants=v2"));
        assert_eq!(request.authorization.as_deref(), Some("Bearer sk-fixture"));
    }

    let missing = OpenAiAssistantRequest::Threads(ThreadsCall {
        options: ClientOptions::default(),
        operation: ThreadsOperation::Retrieve {
            thread_id: String::from("thread_1"),
            async_requested: false,
        },
    });
    let error = dispatch(
        OperationRequest::OpenAiAssistant(missing),
        transport,
        &mut logger,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        OpenAiAssistantError::Threads(betterlite_openai::threads::error::ThreadsError::MissingApiKey)
    ));
}

async fn runs_case() {
    let transport = Arc::new(Recorder::default());
    for _ in 0..4 {
        transport.queue(200, RUN);
    }
    transport.queue(200, RUN);
    transport.queue_stream(&stream_body());
    transport.queue(500, ERROR_BODY);
    let mut logger = RecordingLogger::default();
    let buffered = OpenAiAssistantRequest::Runs(RunsCall {
        options: options(KEY),
        dispatch: RunDispatch::default(),
        request: run_request(),
    });
    let polled = dispatch(
        OperationRequest::OpenAiAssistant(buffered),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Runs(RunOutcome::Polled(
        kind,
        reply,
    ))) = polled
    else {
        panic!("a buffered run did not return a polled reply")
    };
    assert_eq!(kind, ClientKind::Sync);
    assert_eq!(reply.status, "completed");

    let asynchronous = OpenAiAssistantRequest::Runs(RunsCall {
        options: options(KEY),
        dispatch: RunDispatch {
            arun_thread: true,
            stream: false,
        },
        request: run_request(),
    });
    let polled = dispatch(
        OperationRequest::OpenAiAssistant(asynchronous),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Runs(RunOutcome::Polled(
        kind,
        _,
    ))) = polled
    else {
        panic!("the async facade did not return a polled reply")
    };
    assert_eq!(kind, ClientKind::Async);

    let streaming = OpenAiAssistantRequest::Runs(RunsCall {
        options: options(KEY),
        dispatch: RunDispatch {
            arun_thread: false,
            stream: true,
        },
        request: run_request(),
    });
    let streamed = dispatch(
        OperationRequest::OpenAiAssistant(streaming),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Runs(RunOutcome::Streaming(
        mut events,
    ))) = streamed
    else {
        panic!("a streaming run did not stay a stream")
    };
    let mut names = Vec::new();
    while let Some(event) = events.next().await {
        names.push(event.unwrap().name().to_owned());
    }
    assert_eq!(names, vec!["thread.run.created", "thread.run.completed"]);

    let seen = transport.requests();
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1/runs"));
    assert_eq!(
        seen[0].body.as_ref().unwrap(),
        &json!({
            "assistant_id": "asst_1",
            "additional_instructions": null,
            "instructions": "base",
            "metadata": {"tag": "x"},
            "model": "gpt-4o-mini",
            "stream": false,
            "tools": [{"type": "code_interpreter"}],
        })
    );
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].url, format!("{BASE}/threads/thread_1/runs/run_1"));
    assert_eq!(seen[4].method, "POST");
    assert_eq!(
        seen[4].body.as_ref().unwrap()["stream"],
        Value::Bool(true),
        "the streaming run marks the create body"
    );
    for request in &seen {
        assert_eq!(request.beta.as_deref(), Some("assistants=v2"));
    }

    let failing = OpenAiAssistantRequest::Runs(RunsCall {
        options: options(KEY),
        dispatch: RunDispatch::default(),
        request: run_request(),
    });
    let error = dispatch(
        OperationRequest::OpenAiAssistant(failing),
        transport,
        &mut logger,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        OpenAiAssistantError::Runs(ThreadRunsError::Status(500, _))
    ));
}

async fn decisions_case() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let request: DecisionRequestBody = serde_json::from_str(CHOICE_QUESTION).unwrap();
    let call = OpenAiAssistantRequest::Decisions(DecisionsCall {
        api_base: Some(String::from("https://example.test/v1")),
        api_key: Some(String::from("sk-decisions")),
        operation: DecisionsOperation::Request {
            model: String::from("gpt-4o"),
            request,
        },
    });
    let projected = dispatch(
        OperationRequest::OpenAiAssistant(call),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Decisions(DecisionsReply {
        api_base,
        api_key,
        projection,
    })) = projected
    else {
        panic!("the decisions request did not project")
    };
    assert_eq!(api_base.as_deref(), Some("https://example.test/v1"));
    assert_eq!(api_key.as_deref(), Some("sk-decisions"));
    let DecisionsProjection::Request(wire) = projection else {
        panic!("the request call produced a response projection")
    };
    assert!(matches!(wire, ProviderRequestWire::OpenAi(_)));
    assert_eq!(
        serde_json::to_value(&wire).unwrap(),
        json!({
            "model": "gpt-4o",
            "input": "Check this",
            "questions": [{"type": "choice", "name": "label", "instructions": "Pick",
                "choices": [{"value": "yes"}, {"value": "no"}]}],
            "safety_identifier": "safe-1",
        })
    );

    let defaults = OpenAiAssistantRequest::Decisions(DecisionsCall {
        api_base: None,
        api_key: None,
        operation: DecisionsOperation::Request {
            model: String::from("gpt-4o"),
            request: serde_json::from_str(CHOICE_QUESTION).unwrap(),
        },
    });
    let projected = dispatch(
        OperationRequest::OpenAiAssistant(defaults),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Decisions(reply)) = projected
    else {
        panic!("the default resolution did not project")
    };
    assert_eq!(reply.api_base.as_deref(), Some("https://api.openai.com"));
    assert_eq!(reply.api_key, None);

    let payload = br#"{"model":"gpt-4o","answers":[{"type":"choice","name":"label","choice":"yes","probabilities":[{"value":"yes","probability":0.9},{"value":"no","probability":0.1}],"confidence":0.9}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#;
    let response = OpenAiAssistantRequest::Decisions(DecisionsCall {
        api_base: Some(String::from("https://example.test/v1")),
        api_key: Some(String::from("sk-decisions")),
        operation: DecisionsOperation::Response {
            model: String::from("gpt-4o"),
            request: serde_json::from_str(CHOICE_QUESTION).unwrap(),
            payload: payload.to_vec(),
        },
    });
    let projected = dispatch(
        OperationRequest::OpenAiAssistant(response),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Decisions(DecisionsReply {
        projection: DecisionsProjection::Response(response),
        ..
    })) = projected
    else {
        panic!("the response call did not project its answer")
    };
    assert_eq!(response.model, "gpt-4o");
    assert_eq!(response.usage.total_tokens.0, 5);
    assert_eq!(
        serde_json::to_value(&response).unwrap()["answers"][0]["choice"],
        json!("yes")
    );

    let unsupported: DecisionRequestBody = serde_json::from_str(
        r#"{"input":"Check this","questions":[{"type":"choice","instructions":"Pick","choices":[{"value":"only"}]}]}"#,
    )
    .unwrap();
    let single = OpenAiAssistantRequest::Decisions(DecisionsCall {
        api_base: None,
        api_key: None,
        operation: DecisionsOperation::Request {
            model: String::from("gpt-4o"),
            request: unsupported,
        },
    });
    let error = dispatch(
        OperationRequest::OpenAiAssistant(single),
        transport,
        &mut logger,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        OpenAiAssistantError::DecisionsRequest(RequestConversionError::Unsupported(_))
    ));
    let _ = (openai_request_to_ir, ir_to_openai_response, DecisionResponse::default);
}

async fn realtime_session_case() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let params = QueryParams::from([
        (String::from("model"), Some(String::from("realtime-a"))),
        (
            String::from("intent"),
            Some(String::from("transcription")),
        ),
    ]);
    let connect = OpenAiAssistantRequest::Realtime(RealtimeCall::Connect {
        api_base: Some(String::from("https://example.test/")),
        api_key: Some(SecretString::from(KEY)),
        model: String::from("realtime-a"),
        query_params: Some(params),
        beta_realtime: true,
        ssl_verification: Verification::Enabled,
        max_message_size_bytes: 4096,
    });
    let plan = dispatch(
        OperationRequest::OpenAiAssistant(connect),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Session(OpenAiAssistantPrepared {
        url,
        headers,
        ssl,
        max_message_size_bytes,
        force_transcription_model,
        event_normalizer,
    })) = plan
    else {
        panic!("the session preparation did not return a session")
    };
    assert_eq!(
        url,
        "wss://example.test/v1/realtime?model=realtime-a&intent=transcription"
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-fixture")
    );
    assert_eq!(headers.get("OpenAI-Beta").map(String::as_str), Some("realtime=v1"));
    assert_eq!(ssl, RealtimeSsl::Shared);
    assert_eq!(max_message_size_bytes, 4096);
    assert_eq!(force_transcription_model.as_deref(), Some("realtime-a"));
    assert_eq!(event_normalizer, SessionNormalizerKind::Passthrough);
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(logger.calls[0].api_base, url);
    assert_eq!(logger.calls[0].query_params, params);

    let plain = OpenAiAssistantRequest::Realtime(RealtimeCall::Connect {
        api_base: Some(String::from("http://127.0.0.1:8080/base")),
        api_key: Some(SecretString::from(KEY)),
        model: String::from("realtime-a"),
        query_params: None,
        beta_realtime: false,
        ssl_verification: Verification::Disabled,
        max_message_size_bytes: 1024,
    });
    let plan = dispatch(
        OperationRequest::OpenAiAssistant(plain),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::Session(plan)) = plan else {
        panic!("the plain session preparation did not return a session")
    };
    assert_eq!(plan.url, "ws://127.0.0.1:8080/v1/realtime?model=realtime-a");
    assert_eq!(plan.ssl, RealtimeSsl::None);
    assert_eq!(plan.force_transcription_model, None);
    assert_eq!(plan.headers.get("OpenAI-Beta"), None);

    let no_key = OpenAiAssistantRequest::Realtime(RealtimeCall::Connect {
        api_base: None,
        api_key: None,
        model: String::from("realtime-a"),
        query_params: None,
        beta_realtime: false,
        ssl_verification: Verification::Enabled,
        max_message_size_bytes: 1024,
    });
    let error = dispatch(
        OperationRequest::OpenAiAssistant(no_key),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        OpenAiAssistantError::Realtime(betterlite_openai::realtime::error::RealtimeError::MissingApiKey)
    ));

    let handshake = OpenAiAssistantRequest::Realtime(RealtimeCall::Failure(
        RealtimeFailureOwned::Handshake { status_code: 403 },
    ));
    let action = dispatch(
        OperationRequest::OpenAiAssistant(handshake),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::SessionFailure(action)) = action
    else {
        panic!("a handshake refusal did not return a session failure")
    };
    assert_eq!(action, ClientAction::RefuseHandshake { status_code: 403 });

    let other = OpenAiAssistantRequest::Realtime(RealtimeCall::Failure(
        RealtimeFailureOwned::Other {
            message: String::from("connection reset"),
        },
    ));
    let action = dispatch(
        OperationRequest::OpenAiAssistant(other),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::SessionFailure(action)) = action
    else {
        panic!("a connection failure did not return a session failure")
    };
    assert_eq!(
        action,
        ClientAction::Close(RealtimeClose {
            code: 1011,
            reason: String::from("Internal server error: connection reset"),
        })
    );

    let ignored = OpenAiAssistantRequest::Realtime(RealtimeCall::Failure(
        RealtimeFailureOwned::CloseFailed {
            message: String::from("connection reset"),
            close_error: String::from("already completed"),
        },
    ));
    let action = dispatch(
        OperationRequest::OpenAiAssistant(ignored),
        Arc::clone(&transport),
        &mut logger,
    )
    .await
    .unwrap();
    let OperationResult::OpenAiAssistant(OpenAiAssistantResult::SessionFailure(action)) = action
    else {
        panic!("an ignored close failure did not return a session failure")
    };
    assert!(matches!(action, ClientAction::IgnoreCloseFailure { .. }));

    let unexpected = OpenAiAssistantRequest::Realtime(RealtimeCall::Failure(
        RealtimeFailureOwned::CloseFailed {
            message: String::from("connection reset"),
            close_error: String::from("socket gone"),
        },
    ));
    let error = dispatch(
        OperationRequest::OpenAiAssistant(unexpected),
        transport,
        &mut logger,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        OpenAiAssistantError::Realtime(
            betterlite_openai::realtime::error::RealtimeError::UnexpectedCloseFailure { .. }
        )
    ));
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
    let chat = OperationRequest::OpenAi(crate::openai_dispatch::OpenAiRequest);
    let error = dispatch(chat, transport, &mut logger).await.unwrap_err();
    assert!(matches!(
        error,
        OpenAiAssistantError::UnsupportedOperation
    ));
}
