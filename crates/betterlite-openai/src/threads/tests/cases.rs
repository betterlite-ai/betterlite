#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use crate::assistants::client::{ClientKind, SdkClient};
use crate::threads::error::{ReplyFault, ThreadsError};
use crate::threads::operations::{ClientSetup, ThreadCall, ThreadReply};
use crate::threads::records::{
    ThreadCreate, ThreadMessage, ThreadMessageData, ThreadMessagePage, ThreadRecord,
};

use super::support::{
    BASE, KEY, MESSAGE, MESSAGE_COMPLETED, MESSAGE_EMPTY_STATUS, MESSAGE_EXTRAS,
    MESSAGE_IN_PROGRESS, MESSAGE_NO_STATUS, MESSAGE_NULL_STATUS, PAGE, THREAD, THREAD_NULL,
    THREAD_NULL_PROJECTION, THREAD_NUMERIC, THREAD_NUMERIC_PROJECTION, THREAD_PROJECTION, api_with,
    async_setup, options, parse, request, setup,
};

const CREATE_BODY: &str = r#"{"messages":[{"role":"user","content":"Hey, how's it going?"},{"role":"assistant","content":[{"type":"text","text":"Fine."}]}],"metadata":{"team":"core","n":1}}"#;
const MULTIMODAL: &str = r#"{"role":"user","content":[{"type":"text","text":"what is this?"},{"type":"image_url","image_url":{"url":"https://example.test/a.png","detail":"high"}},{"type":"image_file","image_file":{"file_id":"file-1","detail":null}}],"attachments":[{"file_id":"file-1","tools":[{"type":"code_interpreter"},{"type":"file_search"}]},{}],"metadata":{"n":1}}"#;
const NULL_METADATA: &str = r#"{"role":"user","content":"hi","metadata":null}"#;
const ERROR_BODY: &str = r#"{"error":{"message":"No thread found","type":"invalid_request_error","param":null,"code":null}}"#;

#[derive(Debug)]
enum Scenario {
    CreateThreadInitialMessages,
    RetrieveThread,
    AppendMultimodalMessage,
    ListMessagesNoQuery,
    ThreadMetadata,
    AssistantsBetaDefault,
    ThreadAsyncDispatch,
}

fn thread_create(text: &str) -> ThreadCreate {
    serde_json::from_str(text).unwrap()
}

fn message_data(text: &str) -> ThreadMessageData {
    serde_json::from_str(text).unwrap()
}

fn thread_of(reply: ThreadReply) -> ThreadRecord {
    match reply {
        ThreadReply::Thread(thread) => thread,
        ThreadReply::Message(_) | ThreadReply::Page(_) => panic!("expected a thread"),
    }
}

fn message_of(reply: ThreadReply) -> ThreadMessage {
    match reply {
        ThreadReply::Message(message) => *message,
        ThreadReply::Thread(_) | ThreadReply::Page(_) => panic!("expected a message"),
    }
}

fn page_of(reply: ThreadReply) -> ThreadMessagePage {
    match reply {
        ThreadReply::Page(page) => page,
        ThreadReply::Thread(_) | ThreadReply::Message(_) => panic!("expected a page"),
    }
}

async fn create_thread_initial_messages() {
    create_thread_projection().await;
    configured_timeout().await;
}

async fn create_thread_projection() {
    let (api, transport) = api_with(200, THREAD_NULL);
    let data = thread_create(CREATE_BODY);
    let reply = api
        .call(setup(KEY), ThreadCall::Create(&data))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(thread_of(reply)).unwrap(),
        parse(THREAD_NULL_PROJECTION)
    );
    let url = format!("{BASE}/threads");
    assert_eq!(
        transport.seen(),
        vec![request("POST", &url, Some(CREATE_BODY))]
    );
}

async fn configured_timeout() {
    let (api, transport) = api_with(200, THREAD);
    let data = thread_create(CREATE_BODY);
    let mut timed = setup(KEY);
    timed.options.timeout = Some(Duration::from_millis(2500));
    api.call(timed, ThreadCall::Create(&data)).await.unwrap();
    let url = format!("{BASE}/threads");
    let mut expected = request("POST", &url, Some(CREATE_BODY));
    expected["deadline"] = json!(2.5_f64);
    assert_eq!(transport.seen()[0], expected);
}

async fn retrieve_thread() {
    retrieve_thread_projection().await;
    retrieve_thread_failures().await;
}

async fn retrieve_thread_projection() {
    let (api, transport) = api_with(200, THREAD);
    let reply = api
        .call(setup(KEY), ThreadCall::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(thread_of(reply)).unwrap(),
        parse(THREAD_PROJECTION)
    );
    let url = format!("{BASE}/threads/thread_1");
    assert_eq!(transport.seen(), vec![request("GET", &url, None)]);
    // A path id is quoted like the SDK's quote(safe="!$&'()*+,;=:@"), and LiteLLM's object metadata is kept as it arrived.
    transport.queue(200, THREAD_NUMERIC);
    let quoted = api
        .call(async_setup(KEY), ThreadCall::Retrieve("a/b c"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(thread_of(quoted)).unwrap(),
        parse(THREAD_NUMERIC_PROJECTION)
    );
    assert_eq!(
        transport.seen()[1],
        request("GET", &format!("{BASE}/threads/a%2Fb%20c"), None)
    );
}

async fn retrieve_thread_failures() {
    status_error_is_preserved().await;
    request_errors_are_typed().await;
}

fn status_of(error: ThreadsError) -> (u16, Vec<u8>) {
    match error {
        ThreadsError::Status { status, body } => (status, body),
        ThreadsError::MissingApiKey
        | ThreadsError::BaseUrl { .. }
        | ThreadsError::Header { .. }
        | ThreadsError::EmptyThreadId
        | ThreadsError::DotSegment { .. }
        | ThreadsError::Encode { .. }
        | ThreadsError::Transport { .. }
        | ThreadsError::Reply { .. } => panic!("expected a status error"),
    }
}

async fn status_error_is_preserved() {
    let (api, transport) = api_with(404, ERROR_BODY);
    let missing = api
        .call(async_setup(KEY), ThreadCall::Retrieve("thread_missing"))
        .await
        .unwrap_err();
    let (status, body) = status_of(missing);
    assert_eq!(status, 404);
    assert_eq!(body, ERROR_BODY.as_bytes());
    // Only the rejected status call reached the provider.
    assert_eq!(transport.seen().len(), 1);
}

async fn request_errors_are_typed() {
    let (api, _transport) = api_with(200, THREAD);
    let mut anonymous = options(KEY);
    anonymous.api_key = None;
    let unauthenticated = api
        .call(
            ClientSetup {
                options: anonymous,
                supplied: None,
                async_requested: false,
            },
            ThreadCall::Retrieve("thread_1"),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(unauthenticated, ThreadsError::MissingApiKey),
        "{unauthenticated:?}"
    );
    let empty = api
        .call(setup(KEY), ThreadCall::Retrieve(""))
        .await
        .unwrap_err();
    assert!(matches!(empty, ThreadsError::EmptyThreadId), "{empty:?}");
    for bad in [".", ".."] {
        let error = api
            .call(setup(KEY), ThreadCall::Retrieve(bad))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, ThreadsError::DotSegment { segment } if segment.as_str() == bad),
            "{error:?}"
        );
    }
}

async fn append_multimodal_message() {
    message_fields_forwarded().await;
    message_status_defaulting().await;
    message_extras_round_trip().await;
}

async fn message_fields_forwarded() {
    let (api, transport) = api_with(200, MESSAGE_NO_STATUS);
    let data = message_data(MULTIMODAL);
    let reply = api
        .call(
            setup(KEY),
            ThreadCall::AddMessage {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(message_of(reply)).unwrap(),
        parse(MESSAGE_COMPLETED)
    );
    let url = format!("{BASE}/threads/thread_1/messages");
    assert_eq!(
        transport.seen(),
        vec![request("POST", &url, Some(MULTIMODAL))]
    );
    // An explicit null metadata is forwarded as null.
    transport.queue(200, MESSAGE_NO_STATUS);
    let nulls = message_data(NULL_METADATA);
    api.call(
        setup(KEY),
        ThreadCall::AddMessage {
            thread_id: "thread_1",
            data: &nulls,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        transport.seen()[1],
        request("POST", &url, Some(NULL_METADATA))
    );
}

async fn message_status_defaulting() {
    statuses_are_defaulted().await;
    empty_status_is_refused().await;
}

async fn statuses_are_defaulted() {
    let (api, transport) = api_with(200, MESSAGE_IN_PROGRESS);
    transport.queue(200, MESSAGE_NULL_STATUS);
    let data = message_data(MULTIMODAL);
    // An existing status is preserved; a null one is completed like an absent one.
    for expected in [MESSAGE_IN_PROGRESS, MESSAGE_COMPLETED] {
        let reply = api
            .call(
                async_setup(KEY),
                ThreadCall::AddMessage {
                    thread_id: "thread_1",
                    data: &data,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(message_of(reply)).unwrap(),
            parse(expected)
        );
    }
}

async fn empty_status_is_refused() {
    let (api, transport) = api_with(200, MESSAGE_EMPTY_STATUS);
    let data = message_data(MULTIMODAL);
    // An empty-string status is a value the SDK declares no place for, not a missing one.
    let rejected = api
        .call(
            setup(KEY),
            ThreadCall::AddMessage {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            rejected,
            ThreadsError::Reply {
                fault: ReplyFault::Shape,
                ..
            }
        ),
        "{rejected:?}"
    );
    assert_eq!(transport.seen().len(), 1);
}

async fn message_extras_round_trip() {
    let (api, transport) = api_with(200, MESSAGE_EXTRAS);
    let data = message_data(MULTIMODAL);
    let reply = api
        .call(
            async_setup(KEY),
            ThreadCall::AddMessage {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(message_of(reply)).unwrap(),
        parse(MESSAGE_EXTRAS)
    );
    assert_eq!(transport.seen().len(), 1);
}

async fn list_messages_no_query() {
    let (api, transport) = api_with(200, PAGE);
    let reply = api
        .call(setup(KEY), ThreadCall::ListMessages("thread_1"))
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(page_of(reply)).unwrap(), parse(PAGE));
    let url = format!("{BASE}/threads/thread_1/messages");
    assert_eq!(transport.seen(), vec![request("GET", &url, None)]);
    transport.queue(200, PAGE);
    api.call(async_setup(KEY), ThreadCall::ListMessages("thread_1"))
        .await
        .unwrap();
    assert_eq!(transport.seen()[1], request("GET", &url, None));
    // A page keeps extension fields on its messages and on the page itself.
    let extras_page = format!(
        r#"{{"object":"list","data":[{MESSAGE_EXTRAS}],"first_id":"msg_9","last_id":"msg_9","has_more":false}}"#
    );
    transport.queue(200, &extras_page);
    let extras = api
        .call(setup(KEY), ThreadCall::ListMessages("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(page_of(extras)).unwrap(),
        parse(&extras_page)
    );
}

async fn thread_metadata() {
    let (api, transport) = api_with(200, THREAD);
    let empty = thread_create(r#"{"messages":[],"metadata":{}}"#);
    api.call(setup(KEY), ThreadCall::Create(&empty))
        .await
        .unwrap();
    transport.queue(200, THREAD);
    let numeric = thread_create(r#"{"metadata":{"n":1}}"#);
    api.call(setup(KEY), ThreadCall::Create(&numeric))
        .await
        .unwrap();
    transport.queue(200, THREAD);
    let bare = thread_create("{}");
    api.call(setup(KEY), ThreadCall::Create(&bare))
        .await
        .unwrap();
    let url = format!("{BASE}/threads");
    assert_eq!(
        transport.seen(),
        vec![
            request("POST", &url, Some(r#"{"messages":[],"metadata":{}}"#)),
            request("POST", &url, Some(r#"{"metadata":{"n":1}}"#)),
            request("POST", &url, Some("{}")),
        ]
    );
}

async fn assistants_beta_default() {
    let (api, transport) = api_with(200, THREAD);
    transport.queue(200, MESSAGE);
    transport.queue(200, PAGE);
    let bare = thread_create("{}");
    let data = message_data(r#"{"role":"user","content":"hi"}"#);
    let calls = [
        ThreadCall::Create(&bare),
        ThreadCall::Retrieve("thread_1"),
        ThreadCall::AddMessage {
            thread_id: "thread_1",
            data: &data,
        },
        ThreadCall::ListMessages("thread_1"),
    ];
    for call in calls {
        api.call(setup(KEY), call).await.unwrap();
    }
    let betas: Vec<_> = transport
        .seen()
        .iter()
        .map(|seen| seen["beta"].clone())
        .collect();
    assert_eq!(betas, vec![json!("assistants=v2"); 4]);
}

async fn thread_async_dispatch() {
    client_kinds();
    supplied_client_wins().await;
    async_dispatch_success().await;
    async_error_preserved().await;
}

fn client_kinds() {
    let (api, _transport) = api_with(200, THREAD);
    assert_eq!(api.client(setup(KEY)).kind, ClientKind::Sync);
    assert_eq!(api.client(async_setup(KEY)).kind, ClientKind::Async);
}

async fn supplied_client_wins() {
    let (api, transport) = api_with(200, THREAD);
    let supplied = SdkClient {
        kind: ClientKind::Async,
        api_key: Some(SecretString::from("sk-supplied")),
        base_url: Some(String::from("https://supplied.test/v1")),
        timeout: None,
        max_retries: None,
        organization: Some(String::from("org-supplied")),
        transport: Arc::clone(&transport),
    };
    let mut chosen = setup(KEY);
    chosen.supplied = Some(supplied);
    api.call(chosen, ThreadCall::Retrieve("thread_1"))
        .await
        .unwrap();
    let seen = transport.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0]["authorization"].as_str(),
        Some("Bearer sk-supplied")
    );
    assert_eq!(seen[0]["organization"].as_str(), Some("org-supplied"));
    assert_eq!(
        seen[0]["url"].as_str(),
        Some("https://supplied.test/v1/threads/thread_1")
    );
}

async fn async_dispatch_success() {
    let (api, transport) = api_with(200, THREAD);
    transport.queue(200, THREAD);
    transport.queue(200, MESSAGE);
    transport.queue(200, PAGE);
    let bare = thread_create("{}");
    let data = message_data(r#"{"role":"user","content":"hi"}"#);
    let calls = [
        ThreadCall::Create(&bare),
        ThreadCall::Retrieve("thread_1"),
        ThreadCall::AddMessage {
            thread_id: "thread_1",
            data: &data,
        },
        ThreadCall::ListMessages("thread_1"),
    ];
    for call in calls {
        api.call(async_setup(KEY), call).await.unwrap();
    }
    assert_eq!(transport.seen().len(), 4);
}

// The async branch keeps the SDK failure instead of turning it into a reply.
async fn async_error_preserved() {
    let (api, transport) = api_with(404, ERROR_BODY);
    let rejected = api
        .call(async_setup(KEY), ThreadCall::ListMessages("thread_1"))
        .await
        .unwrap_err();
    assert!(
        matches!(rejected, ThreadsError::Status { status: 404, .. }),
        "{rejected:?}"
    );
    assert_eq!(transport.seen().len(), 1);
}

#[rstest]
#[case::create_thread_initial_messages(Scenario::CreateThreadInitialMessages)]
#[case::retrieve_thread(Scenario::RetrieveThread)]
#[case::append_multimodal_message(Scenario::AppendMultimodalMessage)]
#[case::list_messages_no_query(Scenario::ListMessagesNoQuery)]
#[case::thread_metadata(Scenario::ThreadMetadata)]
#[case::assistants_beta_default(Scenario::AssistantsBetaDefault)]
#[case::thread_async_dispatch(Scenario::ThreadAsyncDispatch)]
#[tokio::test]
async fn threads_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::CreateThreadInitialMessages => create_thread_initial_messages().await,
        Scenario::RetrieveThread => retrieve_thread().await,
        Scenario::AppendMultimodalMessage => append_multimodal_message().await,
        Scenario::ListMessagesNoQuery => list_messages_no_query().await,
        Scenario::ThreadMetadata => thread_metadata().await,
        Scenario::AssistantsBetaDefault => assistants_beta_default().await,
        Scenario::ThreadAsyncDispatch => thread_async_dispatch().await,
    }
}
