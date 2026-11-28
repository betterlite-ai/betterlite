use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::bedrock_embedding_io::transport::Timeout;
use betterlite_bedrock::count_call::counter::CountTokensCall;
use betterlite_bedrock::embedding_call::{EmbeddingEnvironment, ParamMap};
use betterlite_bedrock::rerank::plan::RerankEnvironment;
use betterlite_bedrock::transcription::binding::TranscriptionCall;
use betterlite_openai::api::rerank_contracts::records::RerankRequest;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::bedrock_image_dispatch::call::BedrockImageOperation;

// The closed family enum is the only entry: an operation outside the five families is reported, never guessed
pub enum BedrockOtherOperation<'call, 'cost, 'context, H, B> {
    Embedding(EmbeddingOperation<'call>),
    EmbeddingStatus(EmbeddingStatusOperation<'call>),
    Rerank(RerankOperation<'call>),
    Count(CountOperation<'call, H>),
    Transcription(TranscriptionOperation<'call, B>),
    Image(BedrockImageOperation<'call, 'cost, 'context>),
    Unsupported { operation: &'static str },
}

// LiteLLM's embedding arm wraps a bare string input into a one-element list before its handler runs
pub enum EmbeddingInput<'call> {
    Text(&'call str),
    Multiple(&'call [String]),
}

impl EmbeddingInput<'_> {
    #[must_use]
    pub fn list(&self) -> Cow<'_, [String]> {
        match self {
            Self::Text(text) => Cow::Owned(vec![(*text).to_owned()]),
            Self::Multiple(items) => Cow::Borrowed(items),
        }
    }
}

// The call the embedding arm hands its leaf; the caller's headers, identity, timeout, async flag and environment all travel unchanged
pub struct EmbeddingOperation<'call> {
    pub model: &'call str,
    pub input: EmbeddingInput<'call>,
    pub optional_params: ParamMap,
    pub api_base: Option<&'call str>,
    pub extra_headers: Option<&'call [(String, String)]>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: Option<Timeout>,
    pub aembedding: bool,
    pub drop_params: bool,
    pub environment: EmbeddingEnvironment<'call>,
}

// B632's status call; optional_params is read and popped in place, so the caller keeps ownership
pub struct EmbeddingStatusOperation<'call> {
    pub invocation_arn: &'call str,
    pub region: &'call str,
    pub optional_params: &'call mut BTreeMap<String, Box<RawValue>>,
    pub environment: &'call BTreeMap<String, String>,
    pub now: SystemTime,
}

// The rerank arm's call; api_base and extra_headers are the values the arm resolved and merged
pub struct RerankOperation<'call> {
    pub request: RerankRequest,
    pub optional_params: ParamMap,
    pub api_base: Option<&'call str>,
    pub extra_headers: Option<&'call [(String, String)]>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: Option<Duration>,
    pub is_async: bool,
    pub environment: RerankEnvironment<'call>,
    pub now: SystemTime,
    pub generated_id: &'call str,
}

// The count arm's call plus the handler it resolved; no handler is what LiteLLM reports as unavailable
pub struct CountOperation<'call, H> {
    pub call: CountTokensCall<'call>,
    pub handler: Option<H>,
}

// The typed file a transcription caller carries: the bytes of an upload, or those bytes under the filename that names their format
pub enum TranscriptionFile<'call> {
    Bytes(&'call [u8]),
    Multipart {
        filename: &'call str,
        content: &'call [u8],
    },
}

// The transcription arm's call, its typed file and the binding it reached: the native binding or the caller's stand-in
pub struct TranscriptionOperation<'call, B> {
    pub call: TranscriptionCall<'call>,
    pub file: TranscriptionFile<'call>,
    pub asynchronous: bool,
    pub binding: B,
}
