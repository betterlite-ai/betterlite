use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::failure::ClientAction;
use betterlite_openai::realtime::ssl::RealtimeSsl;
use betterlite_openai::threads::operations::ThreadReply;

use self::call::{AssistantsCall, DecisionsCall, RealtimeCall, RunsCall, ThreadsCall};
use self::reply::{AssistantsReply, DecisionsReply, RunOutcome};

pub mod call;
pub mod error;
pub mod execute;
pub mod reply;

// The five completed leaf operations, installed as the payload of the provider operation enum.
#[derive(Debug, Clone)]
pub enum OpenAiAssistantRequest {
    Assistants(AssistantsCall),
    Threads(ThreadsCall),
    Runs(RunsCall),
    Decisions(DecisionsCall),
    Realtime(RealtimeCall),
}

pub enum OpenAiAssistantResult {
    Assistants(AssistantsReply),
    Threads(ThreadReply),
    Runs(RunOutcome),
    Decisions(DecisionsReply),
    Session(OpenAiAssistantPrepared),
    SessionFailure(ClientAction),
}

// The prepared realtime session the composition installs; a handler that installs a normalizer records only its presence here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiAssistantPrepared {
    pub url: String,
    pub headers: Headers,
    pub ssl: RealtimeSsl,
    pub max_message_size_bytes: usize,
    pub force_transcription_model: Option<String>,
    pub event_normalizer: SessionNormalizerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionNormalizerKind {
    Passthrough,
    Installed,
}

mod tests;
