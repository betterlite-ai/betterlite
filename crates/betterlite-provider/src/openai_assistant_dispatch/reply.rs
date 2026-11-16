use betterlite_models::decisions::openai::DecisionResponse;
use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::assistants::records::{
    AssistantDeleted, AssistantObject, AssistantPage,
};
use betterlite_openai::decisions::transformation::wire::ProviderRequestWire;
use betterlite_openai::thread_runs::records::RunReply;
use betterlite_openai::thread_runs::stream::RunEventStream;
use secrecy::SecretString;

#[derive(Debug)]
pub enum AssistantsReply {
    Page(AssistantPage),
    Assistant(AssistantObject),
    Deleted(AssistantDeleted),
}

// The buffered and streaming runs stay distinct, and each names the facade it was built for.
pub enum RunOutcome {
    Polled(ClientKind, Box<RunReply>),
    Streaming(RunEventStream),
}

#[derive(Debug)]
pub enum DecisionsProjection {
    Request(ProviderRequestWire),
    Response(Box<DecisionResponse>),
}

#[derive(Debug)]
pub struct DecisionsReply {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub projection: DecisionsProjection,
}
