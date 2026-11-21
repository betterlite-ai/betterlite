use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::assistants::operations::{AssistantsRunsApi, RunCall, RunDispatch};
use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi, ClientRequest, DefinitionCall, DefinitionReply,
};
use betterlite_azure::azure_message_attachments::operations::{
    MessageCall, MessageReply, MessagesApi,
};
use betterlite_azure::azure_thread_messages::operations::{ThreadMessagesApi, ThreadOperation};
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_openai::thread_runs::records::RunRequest;
use betterlite_openai::threads::records::{ThreadCreate, ThreadMessageData};
use futures_util::StreamExt as _;
use secrecy::SecretString;

use super::fixture::{AssistantsFake, NoSources, Planned, parse};
use crate::azure_resource_dispatch::assistants::{
    self, DefinitionsDispatch, MessagesDispatch, RunOutcome, RunsDispatch, ThreadsDispatch,
};
use crate::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};

const KEY: &str = "sk-fixture";
const BASE: &str = "https://example.test";
const VERSION: &str = "2024-02-01";
const PAGE: &str = r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
const THREAD: &str =
    r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"a"}}"#;
const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hello","annotations":[]}}],"status":"completed"}"#;
const QUEUED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_1","status":"queued","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}"#;
const COMPLETED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"completed_at":1700000005,"assistant_id":"asst_1","thread_id":"thread_1","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}"#;
const STREAM_BODY: &str = concat!(
    "event: thread.run.created\n",
    "data: {\"id\":\"run_1\",\"object\":\"thread.run\",\"created_at\":1700000000,\"assistant_id\":\"asst_1\",\"thread_id\":\"thread_1\",\"status\":\"queued\",\"instructions\":\"base\",\"model\":\"gpt-4o-mini\",\"parallel_tool_calls\":true,\"tools\":[]}\n",
    "\n",
    "event: thread.run.completed\n",
    "data: {\"id\":\"run_1\",\"object\":\"thread.run\",\"created_at\":1700000000,\"completed_at\":1700000005,\"assistant_id\":\"asst_1\",\"thread_id\":\"thread_1\",\"status\":\"completed\",\"instructions\":\"base\",\"model\":\"gpt-4o-mini\",\"parallel_tool_calls\":true,\"tools\":[]}\n",
    "\n",
    "data: [DONE]\n",
    "\n",
);
const ERROR_BODY: &str = r#"{"error":{"message":"nope","type":"invalid_request_error"}}"#;

struct Fixture {
    settings: Settings,
    environment: BTreeMap<String, String>,
    sources: NoSources,
    params: LiteLlmParams,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    api_version: Option<String>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            settings: Settings {
                default_max_retries: 2,
                azure_default_api_version: String::from("2025-02-01-preview"),
            },
            environment: BTreeMap::new(),
            sources: NoSources,
            params: LiteLlmParams {
                azure_ad_token: None,
                auth: AuthParams::default(),
                max_retries: None,
                timeout: None,
            },
            api_key: Some(SecretString::from(KEY)),
            api_base: Some(String::from(BASE)),
            api_version: Some(String::from(VERSION)),
        }
    }

    fn request(&self) -> ClientRequest<'_> {
        ClientRequest {
            params: self.params.clone(),
            provider: None,
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            api_version: self.api_version.clone(),
            azure_ad_token: None,
            timeout: 600.0,
            max_retries: None,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
            settings: &self.settings,
            environment: &self.environment,
            sources: &self.sources,
        }
    }
}

async fn definitions_rows() {
    let fake = Arc::new(AssistantsFake::new(vec![Planned::json(200, PAGE)]));
    let fixture = Fixture::new();
    let request = fixture.request();
    let api = AssistantsApi {
        transport: Arc::clone(&fake),
    };
    let result = dispatch(AzureResourceOperation::AssistantDefinitions(
        assistants::definitions(DefinitionsDispatch {
            api: &api,
            request: &request,
            supplied: None,
            flag: None,
            call: DefinitionCall::List,
        }),
    ))
    .await
    .expect("the list reaches its leaf");
    let AzureResourceResult::AssistantDefinitions(reply) = result else {
        panic!("the definitions dispatch returns its reply");
    };
    let DefinitionReply::Page(page) = reply.reply else {
        panic!("a list returns the assistant page");
    };
    assert!(page.data.is_empty());
    assert_eq!(
        fake.requests()[0].url,
        "https://example.test/openai/assistants?api-version=2024-02-01"
    );

    let create: betterlite_openai::assistants::records::AssistantCreate =
        parse(r#"{"model":"gpt-4o"}"#);
    let failed = AssistantsFake::new(vec![Planned::json(404, ERROR_BODY)]);
    let failing_api = AssistantsApi {
        transport: Arc::new(failed),
    };
    let error = super::fixture::leaf_error(
        dispatch(AzureResourceOperation::AssistantDefinitions(
            assistants::definitions(DefinitionsDispatch {
                api: &failing_api,
                request: &request,
                supplied: None,
                flag: None,
                call: DefinitionCall::Create(&create),
            }),
        ))
        .await,
    );
    assert!(matches!(error, AzureResourceError::AssistantDefinitions(_)));
}

async fn threads_rows() {
    let fake = Arc::new(AssistantsFake::new(vec![Planned::json(200, THREAD)]));
    let fixture = Fixture::new();
    let request = fixture.request();
    let api = ThreadMessagesApi {
        transport: Arc::clone(&fake),
    };
    let create: ThreadCreate = parse(r#"{"messages":[{"role":"user","content":"hello"}]}"#);
    let result = dispatch(AzureResourceOperation::AssistantThreads(
        assistants::threads(ThreadsDispatch {
            api: &api,
            request: &request,
            supplied: None,
            flag: None,
            call: ThreadOperation::Create(&create),
        }),
    ))
    .await
    .expect("the create reaches its leaf");
    let AzureResourceResult::AssistantThreads(reply) = result else {
        panic!("the threads dispatch returns its reply");
    };
    assert_eq!(reply.thread.id, "thread_1");
    let seen = fake.requests();
    assert_eq!(
        seen[0].url,
        "https://example.test/openai/threads?api-version=2024-02-01"
    );
    assert_eq!(seen[0].method, "POST");
}

async fn messages_rows() {
    let fake = Arc::new(AssistantsFake::new(vec![Planned::json(200, MESSAGE)]));
    let fixture = Fixture::new();
    let request = fixture.request();
    let api = MessagesApi {
        assistants: AssistantsApi {
            transport: Arc::clone(&fake),
        },
    };
    let data: ThreadMessageData = parse(r#"{"role":"user","content":"hello"}"#);
    let result = dispatch(AzureResourceOperation::AssistantMessages(
        assistants::messages(MessagesDispatch {
            api: &api,
            request: &request,
            supplied: None,
            flag: None,
            call: MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        }),
    ))
    .await
    .expect("the append reaches its leaf");
    let AzureResourceResult::AssistantMessages(reply) = result else {
        panic!("the messages dispatch returns its reply");
    };
    let MessageReply::Message(message) = reply.reply else {
        panic!("an append returns the message");
    };
    assert_eq!(message.id, "msg_1");
    assert_eq!(
        fake.requests()[0].url,
        "https://example.test/openai/threads/thread_1/messages?api-version=2024-02-01"
    );
}

fn run_request() -> RunRequest {
    parse(
        r#"{"thread_id":"thread_1","assistant_id":"asst_1","model":"gpt-4o-mini","instructions":"base"}"#,
    )
}

async fn runs_rows() {
    let fake = Arc::new(AssistantsFake::new(vec![
        Planned::polled(QUEUED),
        Planned::json(200, COMPLETED),
    ]));
    let fixture = Fixture::new();
    let request = fixture.request();
    let api = AssistantsRunsApi {
        transport: Arc::clone(&fake),
    };
    let run = run_request();
    let result = dispatch(AzureResourceOperation::AssistantRuns(assistants::runs(
        RunsDispatch {
            api: &api,
            request: &request,
            supplied: None,
            call: RunCall {
                dispatch: RunDispatch::default(),
                run: &run,
                handler: None,
            },
        },
    )))
    .await
    .expect("the run reaches its leaf");
    let AzureResourceResult::AssistantRuns(RunOutcome::Polled(_kind, reply)) = result else {
        panic!("a polled run returns its reply");
    };
    assert_eq!(reply.id, "run_1");
    assert_eq!(reply.status, "completed");
    let seen = fake.requests();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[0]
            .url
            .contains("/openai/threads/thread_1/runs?api-version=")
    );
    assert!(
        seen[1]
            .url
            .contains("/openai/threads/thread_1/runs/run_1?api-version=")
    );

    let streaming = Arc::new(AssistantsFake::new(vec![Planned::json(200, STREAM_BODY)]));
    let streaming_api = AssistantsRunsApi {
        transport: Arc::clone(&streaming),
    };
    let streamed = dispatch(AzureResourceOperation::AssistantRuns(assistants::runs(
        RunsDispatch {
            api: &streaming_api,
            request: &request,
            supplied: None,
            call: RunCall {
                dispatch: RunDispatch {
                    arun_thread: None,
                    stream: Some(true),
                },
                run: &run,
                handler: None,
            },
        },
    )))
    .await
    .expect("the streamed run reaches its leaf");
    let AzureResourceResult::AssistantRuns(RunOutcome::Streaming(_kind, events)) = streamed else {
        panic!("a streaming run returns its event stream");
    };
    let mut events = events.await.expect("the stream opens");
    let mut names = Vec::new();
    while let Some(event) = events.next().await {
        names.push(event.expect("the fixture events decode").name().to_owned());
    }
    assert_eq!(names, vec!["thread.run.created", "thread.run.completed"]);

    let failing = Arc::new(AssistantsFake::new(vec![Planned::json(400, ERROR_BODY)]));
    let failing_api = AssistantsRunsApi {
        transport: Arc::clone(&failing),
    };
    let error = super::fixture::leaf_error(
        dispatch(AzureResourceOperation::AssistantRuns(assistants::runs(
            RunsDispatch {
                api: &failing_api,
                request: &request,
                supplied: None,
                call: RunCall {
                    dispatch: RunDispatch::default(),
                    run: &run,
                    handler: None,
                },
            },
        )))
        .await,
    );
    assert!(matches!(error, AzureResourceError::AssistantRuns(_)));
}

pub(super) async fn check() {
    definitions_rows().await;
    threads_rows().await;
    messages_rows().await;
    runs_rows().await;
}
