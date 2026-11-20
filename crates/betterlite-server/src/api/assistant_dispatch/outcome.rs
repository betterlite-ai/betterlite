use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_openai::assistants::records::{AssistantDeleted, AssistantObject, AssistantPage};
use betterlite_openai::threads::records::{ThreadMessage, ThreadMessagePage, ThreadRecord};

// The record the proxy returns for each installed operation, before its route serializes it.
#[derive(Debug)]
pub enum AssistantOutcome<S> {
    Assistant(Box<AssistantObject>),
    Assistants(AssistantPage),
    Deleted(AssistantDeleted),
    Thread(Option<ThreadRecord>),
    Message(Box<ThreadMessage>),
    Messages(Box<ThreadMessagePage>),
    Run(RunOutcome<S>),
}
