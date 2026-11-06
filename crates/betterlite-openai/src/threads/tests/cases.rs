#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use serde_json::json;

use crate::assistants::client::ClientKind;
use crate::threads::error::{ReplyFault, ThreadsError};
use crate::threads::operations::{ThreadCall, ThreadReply, dispatch_kind};
use crate::threads::records::{
    ThreadCreate, ThreadMessage, ThreadMessageData, ThreadMessagePage, ThreadRecord,
};

use super::support::{
    BASE, KEY, MESSAGE, MESSAGE_NO_STATUS, PAGE, THREAD, THREAD_PROJECTION, api_with, options,
    parse, request,
};

const CREATE_BODY: &str = r#"{"messages":[{"role":"user","content":"Hey, how's it going?"},{"role":"assistant","content":[{"type":"text","text":"Fine."}]}]}"#;
const MULTIMODAL: &str = r#"{"role":"user","content":[{"type":"text","text":"what is this?"},{"type":"image_url","image_url":{"url":"https://example.test/a.png","detail":"high"}},{"type":"image_file","image_file":{"file_id":"file-1","detail":null}}],"attachments":[{"file_id":"file-1","tools":[{"type":"code_interpreter"},{"type":"file_search"}]},{}],"metadata":null}"#;
const MESSAGE_COMPLETED: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"completed"}"#;
const MESSAGE_IN_PROGRESS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"in_progress"}"#;
const MESSAGE_NULL_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":null}"#;
const MESSAGE_EMPTY_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":""}"#;

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
        ThreadReply::Message(message) => message,
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
    let (api, transport) = api_with(200, THREAD);
    let data = thread_create(CREATE_BODY);
    let reply = api
        .call(options(KEY), None, false, ThreadCall::Create(&data))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(thread_of(reply)).unwrap(),
        parse(THREAD_PROJECTION)
    );
    let url = format!("{BASE}/threads");
    assert_eq!(
        transport.seen(),
        vec![request("POST", &url, Some(CREATE_BODY))]
    );
    // The caller's timeout reaches the request; otherwise the SDK's 600s default applies.
    let mut timed = options(KEY);
    timed.timeout = Some(Duration::from_millis(2500));
    transport.queue(200, THREAD);
    api.call(timed, None, false, ThreadCall::Create(&data))
        .await
        .unwrap();
    let mut expected = request("POST", &url, Some(CREATE_BODY));
    expected["deadline"] = json!(2.5_f64);
    assert_eq!(transport.seen()[1], expected);
}

async fn retrieve_thread() {
    retrieve_thread_projection().await;
    retrieve_thread_failures().await;
}

async fn retrieve_thread_projection() {
    let (api, transport) = api_with(200, THREAD);
    let reply = api
        .call(options(KEY), None, false, ThreadCall::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(thread_of(reply)).unwrap(),
        parse(THREAD_PROJECTION)
    );
    let url = format!("{BASE}/threads/thread_1");
    assert_eq!(transport.seen(), vec![request("GET", &url, None)]);
    // A path id is quoted like the SDK's quote(safe="!$&'()*+,;=:@"), and a dot segment never reaches the wire.
    transport.queue(200, THREAD);
    api.call(options(KEY), None, true, ThreadCall::Retrieve("a/b c"))
        .await
        .unwrap();
    assert_eq!(
        transport.seen()[1],
        request("GET", &format!("{BASE}/threads/a%2Fb%20c"), None)
    );
}

async fn retrieve_thread_failures() {
    let (api, transport) = api_with(404, r#"{"error":{"message":"No thread found"}}"#);
    let missing = api
        .call(
            options(KEY),
            None,
            false,
            ThreadCall::Retrieve("thread_missing"),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&missing, ThreadsError::Status { status: 404, .. }),
        "{missing:?}"
    );
    let mut anonymous = options(KEY);
    anonymous.api_key = None;
    let unauthenticated = api
        .call(anonymous, None, false, ThreadCall::Retrieve("thread_1"))
        .await
        .unwrap_err();
    assert!(
        matches!(unauthenticated, ThreadsError::MissingApiKey),
        "{unauthenticated:?}"
    );
    for bad in ["", ".."] {
        let error = api
            .call(options(KEY), None, false, ThreadCall::Retrieve(bad))
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                ThreadsError::EmptyThreadId | ThreadsError::DotSegment { .. }
            ),
            "{bad:?}"
        );
    }
    // Only the rejected status call reached the provider.
    assert_eq!(transport.seen().len(), 1);
}

async fn append_multimodal_message() {
    message_fields_forwarded().await;
    message_status_defaulting().await;
}

async fn message_fields_forwarded() {
    let (api, transport) = api_with(200, MESSAGE_NO_STATUS);
    let data = message_data(MULTIMODAL);
    let call = ThreadCall::AddMessage {
        thread_id: "thread_1",
        data: &data,
    };
    let reply = api.call(options(KEY), None, false, call).await.unwrap();
    assert_eq!(
        serde_json::to_value(message_of(reply)).unwrap(),
        parse(MESSAGE_COMPLETED)
    );
    let url = format!("{BASE}/threads/thread_1/messages");
    assert_eq!(
        transport.seen(),
        vec![request("POST", &url, Some(MULTIMODAL))]
    );
}

async fn message_status_defaulting() {
    let (api, transport) = api_with(200, MESSAGE_IN_PROGRESS);
    transport.queue(200, MESSAGE_NULL_STATUS);
    let data = message_data(MULTIMODAL);
    // An existing status is preserved; a null one is completed like an absent one.
    for expected in [MESSAGE_IN_PROGRESS, MESSAGE_COMPLETED] {
        let call = ThreadCall::AddMessage {
            thread_id: "thread_1",
            data: &data,
        };
        let reply = api.call(options(KEY), None, true, call).await.unwrap();
        assert_eq!(
            serde_json::to_value(message_of(reply)).unwrap(),
            parse(expected)
        );
    }
    // An empty-string status is a value the SDK declares no place for, not a missing one.
    transport.queue(200, MESSAGE_EMPTY_STATUS);
    let call = ThreadCall::AddMessage {
        thread_id: "thread_1",
        data: &data,
    };
    let rejected = api.call(options(KEY), None, false, call).await.unwrap_err();
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
}

async fn list_messages_no_query() {
    let (api, transport) = api_with(200, PAGE);
    let reply = api
        .call(
            options(KEY),
            None,
            false,
            ThreadCall::ListMessages("thread_1"),
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(page_of(reply)).unwrap(), parse(PAGE));
    let url = format!("{BASE}/threads/thread_1/messages");
    assert_eq!(transport.seen(), vec![request("GET", &url, None)]);
    transport.queue(200, PAGE);
    api.call(
        options(KEY),
        None,
        true,
        ThreadCall::ListMessages("thread_1"),
    )
    .await
    .unwrap();
    assert_eq!(transport.seen()[1], request("GET", &url, None));
}

async fn thread_metadata() {
    let (api, transport) = api_with(200, THREAD);
    let empty = thread_create(r#"{"messages":[],"metadata":{}}"#);
    api.call(options(KEY), None, false, ThreadCall::Create(&empty))
        .await
        .unwrap();
    transport.queue(200, THREAD);
    let bare = thread_create("{}");
    api.call(options(KEY), None, false, ThreadCall::Create(&bare))
        .await
        .unwrap();
    let url = format!("{BASE}/threads");
    assert_eq!(
        transport.seen(),
        vec![
            request("POST", &url, Some(r#"{"messages":[],"metadata":{}}"#)),
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
        api.call(options(KEY), None, false, call).await.unwrap();
    }
    let betas: Vec<_> = transport
        .seen()
        .iter()
        .map(|seen| seen["beta"].clone())
        .collect();
    assert_eq!(betas, vec![json!("assistants=v2"); 4]);
}

async fn thread_async_dispatch() {
    assert_eq!(dispatch_kind(true), ClientKind::Async);
    assert_eq!(dispatch_kind(false), ClientKind::Sync);
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
        api.call(options(KEY), None, true, call).await.unwrap();
    }
    assert_eq!(
        transport.seen(),
        vec![
            request("POST", &format!("{BASE}/threads"), Some("{}")),
            request("GET", &format!("{BASE}/threads/thread_1"), None),
            request(
                "POST",
                &format!("{BASE}/threads/thread_1/messages"),
                Some(r#"{"role":"user","content":"hi"}"#)
            ),
            request("GET", &format!("{BASE}/threads/thread_1/messages"), None),
        ]
    );
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
