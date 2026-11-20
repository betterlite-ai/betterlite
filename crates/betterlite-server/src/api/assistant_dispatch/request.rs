use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_openai::thread_runs::stream::RunEventHandler;

use super::client::AssistantClient;
use super::operation::AssistantOperation;

// One call of one installed wrapper: the proxy's data dict, the path argument it passes explicitly, and
// the client object it was handed (None on every HTTP route).
pub struct AssistantRequest<'request> {
    pub operation: AssistantOperation,
    pub data: &'request BTreeMap<String, Json>,
    pub thread_id: Option<&'request str>,
    pub assistant_id: Option<&'request str>,
    pub client: Option<AssistantClient>,
    pub event_handler: Option<Arc<dyn RunEventHandler>>,
}

impl<'request> AssistantRequest<'request> {
    // the path value the operation takes, if any
    #[must_use]
    pub const fn path_value(&self) -> Option<&'request str> {
        match self.operation {
            AssistantOperation::DeleteAssistant => self.assistant_id,
            AssistantOperation::GetThread
            | AssistantOperation::CreateMessage
            | AssistantOperation::ListMessages
            | AssistantOperation::RunThread => self.thread_id,
            AssistantOperation::CreateAssistant
            | AssistantOperation::ListAssistants
            | AssistantOperation::CreateThread => None,
        }
    }
}
