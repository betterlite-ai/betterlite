use std::collections::BTreeMap;

use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::client::SuppliedClass;
use super::super::operation::AssistantOperation;
use super::super::outcome::AssistantOutcome;
use super::*;

// the record each installed operation answers with
#[derive(Debug)]
enum Answered {
    Assistant,
    Assistants,
    Deleted,
    Thread,
    Message,
    Messages,
    Polled,
    Streaming,
}

// the request one operation sends, as far as its case checks it
struct Posted {
    method: &'static str,
    url: Option<&'static str>,
    authorization: Option<&'static str>,
}

// one installed operation, its body, its route and what the mock provider answers
struct OpCase {
    operation: AssistantOperation,
    fields: BTreeMap<String, Json>,
    route: Route<'static>,
    replies: &'static [(u16, &'static str)],
    posted: Option<Posted>,
    answered: Answered,
}

// every assistant wrapper the router installs, each reaching its provider method
#[rstest]
#[case::create_assistant(OpCase {
    operation: AssistantOperation::CreateAssistant,
    fields: data(&[
        ("model", text("gpt-4o-mini")),
        ("name", text("helper")),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route::default(),
    replies: &[(200, ASSISTANT)],
    posted: Some(Posted {
        method: "POST",
        url: Some("https://mock.test/v1/assistants"),
        authorization: Some("Bearer sk-test"),
    }),
    answered: Answered::Assistant,
})]
#[case::list_assistants(OpCase {
    operation: AssistantOperation::ListAssistants,
    fields: openai_data(),
    route: Route {
        thread_id: None,
        assistant_id: None,
        client: Some(supplied(
            SuppliedClass::AsyncOpenAi,
            "sk-supplied",
            MOCK_BASE,
            None,
        )),
    },
    replies: &[(200, PAGE)],
    posted: Some(Posted {
        method: "GET",
        url: None,
        // a supplied client's own key reaches the SDK owner unchanged
        authorization: Some("Bearer sk-supplied"),
    }),
    answered: Answered::Assistants,
})]
#[case::delete_assistant(OpCase {
    operation: AssistantOperation::DeleteAssistant,
    fields: openai_data(),
    route: Route {
        thread_id: None,
        assistant_id: Some("asst_a"),
        client: None,
    },
    replies: &[(200, DELETED)],
    posted: Some(Posted {
        method: "DELETE",
        url: Some("https://mock.test/v1/assistants/asst_a"),
        authorization: None,
    }),
    answered: Answered::Deleted,
})]
#[case::create_thread(OpCase {
    operation: AssistantOperation::CreateThread,
    fields: openai_data(),
    route: Route::default(),
    replies: &[(200, THREAD)],
    posted: Some(Posted {
        method: "POST",
        url: Some("https://mock.test/v1/threads"),
        authorization: None,
    }),
    answered: Answered::Thread,
})]
#[case::get_thread(OpCase {
    operation: AssistantOperation::GetThread,
    fields: openai_data(),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: Some(supplied(
            SuppliedClass::AsyncOpenAi,
            "sk-supplied",
            MOCK_BASE,
            None,
        )),
    },
    replies: &[(200, THREAD)],
    posted: Some(Posted {
        method: "GET",
        url: Some("https://mock.test/v1/threads/thread_a"),
        // a supplied client's own key reaches the thread facade unchanged
        authorization: Some("Bearer sk-supplied"),
    }),
    answered: Answered::Thread,
})]
#[case::create_message(OpCase {
    operation: AssistantOperation::CreateMessage,
    fields: data(&[
        ("role", text("user")),
        ("content", text("hello")),
        // a body client key is not forwarded to the wrapper, so it never binds the client argument
        ("client", object(&[("api_key", text("sk-body"))])),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, MESSAGE)],
    posted: Some(Posted {
        method: "POST",
        url: Some("https://mock.test/v1/threads/thread_a/messages"),
        authorization: None,
    }),
    answered: Answered::Message,
})]
#[case::list_messages(OpCase {
    operation: AssistantOperation::ListMessages,
    fields: openai_data(),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, MESSAGE_PAGE)],
    posted: Some(Posted {
        method: "GET",
        url: None,
        authorization: None,
    }),
    answered: Answered::Messages,
})]
#[case::polled_run(OpCase {
    operation: AssistantOperation::RunThread,
    fields: data(&[
        ("assistant_id", text("asst_a")),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, RUN), (200, RUN)],
    posted: Some(Posted {
        method: "POST",
        url: Some("https://mock.test/v1/threads/thread_a/runs"),
        authorization: None,
    }),
    answered: Answered::Polled,
})]
#[case::streamed_run(OpCase {
    operation: AssistantOperation::RunThread,
    fields: data(&[
        ("assistant_id", text("asst_a")),
        ("stream", Json::Bool(true)),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, RUN)],
    posted: None,
    answered: Answered::Streaming,
})]
#[tokio::test]
async fn dispatch_all_assistant_operations(#[case] case: OpCase) {
    let settings = configured("openai", &[]);
    let outcome = call(
        &settings,
        case.operation,
        &case.fields,
        case.route,
        case.replies,
    )
    .await;
    assert_posted(&outcome, case.posted.as_ref());
    assert_answered(outcome, case.answered);
}

// the request the operation sent, or none when the case expects the lazy stream
fn assert_posted(outcome: &OutcomeView, posted: Option<&Posted>) {
    let Some(expected) = posted else {
        // stream=true stays a lazy manager until its owner enters it; no provider request happened yet
        assert!(outcome.seen.is_empty());
        return;
    };
    let seen = outcome.seen.first().expect("the operation sent a request");
    assert_eq!(seen.method, expected.method);
    if let Some(url) = expected.url {
        assert_eq!(seen.url, url);
    }
    if let Some(authorization) = expected.authorization {
        assert_eq!(seen.authorization.as_deref(), Some(authorization));
    }
}

// the record the operation answered with
fn assert_answered(outcome: OutcomeView, answered: Answered) {
    match (answered, outcome.result.expect("the operation answers")) {
        (Answered::Assistant, AssistantOutcome::Assistant(record)) => {
            assert_eq!(record.id, "asst_a");
        }
        (Answered::Assistants, AssistantOutcome::Assistants(page)) => {
            assert_eq!(page.data.first().expect("a listed assistant").id, "asst_a");
        }
        (Answered::Deleted, AssistantOutcome::Deleted(record)) => assert!(record.deleted),
        (Answered::Thread, AssistantOutcome::Thread(Some(record))) => {
            assert_eq!(record.id, "thread_a");
        }
        (Answered::Message, AssistantOutcome::Message(record)) => assert_eq!(record.id, "msg_a"),
        (Answered::Messages, AssistantOutcome::Messages(page)) => {
            assert_eq!(page.data.first().expect("a listed message").id, "msg_a");
        }
        (Answered::Polled, AssistantOutcome::Run(RunOutcome::Polled(reply))) => {
            assert_eq!((*reply).expect("the provider replied").id, "run_a");
        }
        (Answered::Streaming, AssistantOutcome::Run(RunOutcome::Streaming(_))) => {}
        (other, _) => panic!("the operation answered a {other:?} case with another outcome"),
    }
}
