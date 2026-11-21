use serde::{Deserialize, Serialize};

// The eight asynchronous wrappers Router.initialize_assistants_endpoint installs (router.py#L1899-L1908).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantOperation {
    CreateAssistant,
    ListAssistants,
    DeleteAssistant,
    CreateThread,
    GetThread,
    CreateMessage,
    ListMessages,
    RunThread,
}

impl AssistantOperation {
    // the module-level function the factory calls, and the path argument the proxy passes next to **data
    #[must_use]
    pub const fn installed(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::CreateAssistant => ("acreate_assistants", None),
            Self::ListAssistants => ("aget_assistants", None),
            Self::DeleteAssistant => ("adelete_assistant", Some("assistant_id")),
            Self::CreateThread => ("acreate_thread", None),
            Self::GetThread => ("aget_thread", Some("thread_id")),
            Self::CreateMessage => ("a_add_message", Some("thread_id")),
            Self::ListMessages => ("aget_messages", Some("thread_id")),
            Self::RunThread => ("arun_thread", Some("thread_id")),
        }
    }
}
