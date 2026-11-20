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
    // the module-level function the factory hands its merged arguments to; a keyword collision names it
    #[must_use]
    pub const fn function(self) -> &'static str {
        match self {
            Self::CreateAssistant => "acreate_assistants",
            Self::ListAssistants => "aget_assistants",
            Self::DeleteAssistant => "adelete_assistant",
            Self::CreateThread => "acreate_thread",
            Self::GetThread => "aget_thread",
            Self::CreateMessage => "a_add_message",
            Self::ListMessages => "aget_messages",
            Self::RunThread => "arun_thread",
        }
    }

    // the explicit path argument the proxy passes next to **data, mirroring proxy_server.py
    #[must_use]
    pub const fn path_argument(self) -> Option<&'static str> {
        match self {
            Self::DeleteAssistant => Some("assistant_id"),
            Self::GetThread | Self::CreateMessage | Self::ListMessages | Self::RunThread => {
                Some("thread_id")
            }
            Self::CreateAssistant | Self::ListAssistants | Self::CreateThread => None,
        }
    }
}
