use rstest::rstest;

use super::super::error::AssistantDispatchError;
use super::super::operation::AssistantOperation;
use super::*;

// the wrapper's own named arguments: a route without the id leaves it for the interpreter to report
#[rstest]
#[case::role_and_content(
    AssistantOperation::CreateMessage,
    Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    "a_add_message() missing 2 required positional arguments: 'role' and 'content'"
)]
#[case::assistant_id(
    AssistantOperation::RunThread,
    Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    "arun_thread() missing 1 required positional argument: 'assistant_id'"
)]
// the route always carries the path id, so only a caller that builds the request itself reaches this
#[case::delete_id(
    AssistantOperation::DeleteAssistant,
    Route::default(),
    "delete_assistant() missing 1 required positional argument: 'assistant_id'"
)]
#[case::thread_id(
    AssistantOperation::CreateMessage,
    Route::default(),
    "a_add_message() missing 3 required positional arguments: 'thread_id', 'role', and 'content'"
)]
#[tokio::test]
async fn missing_arguments_use_the_interpreter_wording(
    #[case] operation: AssistantOperation,
    #[case] route: Route<'static>,
    #[case] expected: &str,
) {
    let settings = configured("openai", &[]);
    let outcome = call(&settings, operation, &openai_data(), route, &[]).await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the missing argument did not raise");
    };
    assert_eq!(message, expected);
    assert!(outcome.seen.is_empty());
}
