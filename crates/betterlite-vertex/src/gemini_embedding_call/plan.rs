use std::time::Duration;

use crate::endpoint_target::target::TargetProvider;
use crate::gemini_embedding_records::input::{EmbeddingInput, InputEntry, flatten};

use super::call::{EmbeddingCall, Execution};
use super::error::GeminiEmbeddingCallError;

const FILE_PREFIX: &str = "files/";
const FALLBACK_READ: Duration = Duration::from_secs(600);
const FALLBACK_CONNECT: Duration = Duration::from_secs(5);
// what a caller-supplied client applies when the call names no timeout of its own
pub const SUPPLIED_CLIENT_DEADLINE: Duration = FALLBACK_READ;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facade {
    EmbedContent,
    BatchEmbedContents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientSource {
    Supplied,
    Created,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub read: Duration,
    pub connect: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CallPlan {
    pub facade: Facade,
    pub files: Vec<String>,
    pub client: ClientSource,
    // None leaves every request to the supplied client's own timeouts
    pub timeouts: Option<Timeouts>,
}

#[must_use]
pub fn flatten_and_detect_file_refs(input: &EmbeddingInput) -> (Vec<&str>, bool) {
    let flat = flatten(input);
    let has_file_refs = flat.iter().any(|element| element.starts_with(FILE_PREFIX));
    (flat, has_file_refs)
}

// Vertex walks the top level only, so a combined group never reaches the metadata lookups
fn top_level_texts(input: &EmbeddingInput) -> Vec<&str> {
    match input {
        EmbeddingInput::Text(text) => vec![text.as_str()],
        EmbeddingInput::Entries(entries) => entries
            .iter()
            .filter_map(|entry| match entry {
                InputEntry::Text(text) => Some(text.as_str()),
                InputEntry::Group(_) => None,
            })
            .collect(),
    }
}

fn file_references(call: &EmbeddingCall<'_>) -> Result<Vec<String>, GeminiEmbeddingCallError> {
    let candidates = match call.provider {
        TargetProvider::Vertex if call.api_key.is_some() => top_level_texts(call.input),
        TargetProvider::Vertex => Vec::new(),
        TargetProvider::Gemini => {
            let (flat, has_file_refs) = flatten_and_detect_file_refs(call.input);
            if has_file_refs && call.api_key.is_none() {
                return Err(GeminiEmbeddingCallError::MissingApiKey);
            }
            flat
        }
    };
    Ok(candidates
        .into_iter()
        .filter(|element| element.starts_with(FILE_PREFIX))
        .map(str::to_owned)
        .collect())
}

// None falls back to 600 seconds to read and 5 to connect, and the async source's pooled client lands on the same values
fn timeouts_for(timeout: Option<Duration>) -> Timeouts {
    timeout.map_or(
        Timeouts {
            read: FALLBACK_READ,
            connect: FALLBACK_CONNECT,
        },
        |both| Timeouts {
            read: both,
            connect: both,
        },
    )
}

pub fn plan_call(
    call: &EmbeddingCall<'_>,
    client_supplied: bool,
) -> Result<CallPlan, GeminiEmbeddingCallError> {
    let facade = match call.provider {
        TargetProvider::Vertex => Facade::EmbedContent,
        TargetProvider::Gemini => Facade::BatchEmbedContents,
    };
    let files = file_references(call)?;
    let (client, timeouts) = match (call.execution, client_supplied) {
        (Execution::Sync, true) => (ClientSource::Supplied, None),
        (Execution::Sync, false) | (Execution::Async, true | false) => {
            (ClientSource::Created, Some(timeouts_for(call.timeout)))
        }
    };
    Ok(CallPlan {
        facade,
        files,
        client,
        timeouts,
    })
}
