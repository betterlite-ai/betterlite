use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::failure::ClientAction;
use betterlite_openai::realtime::ssl::RealtimeSsl;
use betterlite_openai::threads::operations::ThreadReply;

use self::reply::{AssistantsReply, DecisionsReply, RunOutcome};

pub mod call;
pub mod error;
pub mod execute;
pub mod reply;

// The provider operation payload is a unit marker: the typed call travels beside it, since operation.rs builds and matches the marker alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiAssistantRequest;

pub enum OpenAiAssistantResult {
    Assistants(AssistantsReply),
    Threads(ThreadReply),
    Runs(RunOutcome),
    Decisions(DecisionsReply),
    Session(OpenAiAssistantPrepared),
    SessionFailure(ClientAction),
}

// The prepared realtime session the composition installs; OpenAI installs no event normalizer at the pin, so only its presence is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiAssistantPrepared {
    pub url: String,
    pub headers: Headers,
    pub ssl: RealtimeSsl,
    pub max_message_size_bytes: usize,
    pub force_transcription_model: Option<String>,
    pub event_normalizer_installed: bool,
}

mod tests;
