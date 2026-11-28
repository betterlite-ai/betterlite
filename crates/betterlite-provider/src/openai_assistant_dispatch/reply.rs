use betterlite_http::client_kind::ClientKind;
use betterlite_models::decisions::openai::DecisionResponse;
use betterlite_openai::assistants::records::{AssistantDeleted, AssistantObject, AssistantPage};
use betterlite_openai::decisions::transformation::wire::ProviderRequestWire;
use betterlite_openai::thread_runs::records::RunReply;
use betterlite_openai::thread_runs::stream::RunEventStream;
use secrecy::SecretString;

// the assistant record dwarfs a deletion, so it is boxed to stay within 3x, the limit variant_size_differences enforces
#[derive(Debug)]
pub enum AssistantsReply {
    Page(AssistantPage),
    Assistant(Box<AssistantObject>),
    Deleted(AssistantDeleted),
}

// The buffered and streaming runs stay distinct, and each names the facade it was built for; the stream sends nothing until it is first polled.
pub enum RunOutcome {
    Polled(ClientKind, Box<RunReply>),
    Streaming(ClientKind, RunEventStream),
}

#[derive(Debug)]
pub enum DecisionsProjection {
    // both wires are boxed so neither variant is over 3x the other, the limit variant_size_differences enforces
    Request(Box<ProviderRequestWire>),
    Response(Box<DecisionResponse>),
}

#[derive(Debug)]
pub struct DecisionsReply {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub projection: DecisionsProjection,
}
