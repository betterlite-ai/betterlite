use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use aws_smithy_async::time::SharedTimeSource;
use betterlite_bedrock::sagemaker_runtime::completion::{self as completion_call, CompletionCall};
use betterlite_bedrock::sagemaker_runtime::embedding::{
    self as embedding_call, EmbeddingCall, EmbeddingReply as SagemakerEmbeddingReply,
};
use betterlite_bedrock::sagemaker_runtime::endpoint::ParamMap;
use betterlite_bedrock::sagemaker_runtime::runtime::{
    ChunkStream, SagemakerEnvironment, SagemakerRuntime,
};
use betterlite_bedrock::sagemaker_stream::SagemakerEvents;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::sagemaker_completion::params::Options;
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_prompts::arguments::json::Object as Body;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use tokio::runtime::Handle;
use tokio::task::block_in_place;
use uuid::Uuid;

use super::error::EnterpriseError;
use super::transport::{AwsCredentials, EnterpriseTransport, SharedCredentials, SharedTransport};

// the dependencies B5840 takes as arguments: the pooled client, the credential resolver, the clock and the counter
pub struct SagemakerDeps<'deps> {
    pub transport: &'deps dyn EnterpriseTransport,
    pub credentials: &'deps dyn AwsCredentials,
    pub environment: SagemakerEnvironment,
    pub clock: SharedTimeSource,
    pub counter: &'deps (dyn TextTokenCounter + Sync),
    pub encoding: &'deps (dyn TextEncoding + Sync),
    pub prompts: &'deps PromptFactory,
    pub timeout: Duration,
    pub identifier: fn() -> Uuid,
}

impl SagemakerDeps<'_> {
    fn runtime(&self) -> SagemakerRuntime<'_, SharedTransport<'_>, SharedCredentials<'_>> {
        SagemakerRuntime {
            transport: SharedTransport {
                inner: self.transport,
            },
            credentials: SharedCredentials {
                inner: self.credentials,
            },
            environment: self.environment.clone(),
            clock: self.clock.clone(),
            counter: self.counter,
            timeout: self.timeout,
            identifier: self.identifier,
        }
    }
}

// _complete_sagemaker's fields: the context messages and optional params, the AWS values in optional params
pub struct SagemakerCompletion<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request Options,
    pub signing: &'request ParamMap,
    pub custom_prompt_dict: &'request BTreeMap<String, CustomPromptEntry>,
    pub global_custom_prompt_dict: &'request BTreeMap<String, CustomPromptEntry>,
    pub hf_model_name: Option<&'request str>,
    pub stream: bool,
}

pub struct SagemakerEmbedding<'request> {
    pub model: &'request str,
    pub input: &'request EmbeddingInput,
    pub optional_params: &'request Body,
    pub signing: &'request ParamMap,
}

pub enum SagemakerReply {
    Completion(ChatResponse),
    Stream(SagemakerEvents<ChunkStream>),
}

// _complete_sagemaker: B5840's completion, or its stream decoder when the optional params ask for a stream
pub fn complete(
    deps: &SagemakerDeps<'_>,
    call: &SagemakerCompletion<'_>,
) -> Result<SagemakerReply, EnterpriseError> {
    let inner = CompletionCall {
        model: call.model,
        messages: call.messages,
        optional_params: call.optional_params,
        signing: call.signing,
        custom_prompt_dict: call.custom_prompt_dict,
        global_custom_prompt_dict: call.global_custom_prompt_dict,
        hf_model_name: call.hf_model_name,
        prompts: deps.prompts,
    };
    if call.stream {
        let events = block_in_place(|| {
            Handle::current().block_on(completion_call::completion_stream(&deps.runtime(), &inner))
        })?;
        return Ok(SagemakerReply::Stream(events));
    }
    let reply = block_in_place(|| {
        Handle::current().block_on(completion_call::completion(&deps.runtime(), &inner))
    })?;
    Ok(SagemakerReply::Completion(reply))
}

// the sagemaker embedding arm, whose reply B5961 shapes per model
pub fn embedding(
    deps: &SagemakerDeps<'_>,
    call: &SagemakerEmbedding<'_>,
) -> Result<SagemakerEmbeddingReply, EnterpriseError> {
    let inner = EmbeddingCall {
        model: call.model,
        input: call.input,
        optional_params: call.optional_params,
        signing: call.signing,
        encoding: deps.encoding,
    };
    let reply = block_in_place(|| {
        Handle::current().block_on(embedding_call::embedding(&deps.runtime(), &inner))
    })?;
    Ok(reply)
}

// B196's tokenizer engine plugged into B5960's TextTokenCounter and the cohere embedding TextEncoding
pub struct Tokenizer {
    pub engine: Engine,
}

impl Tokenizer {
    pub fn new() -> Result<Self, EnterpriseError> {
        let vocabulary =
            assets::bundled(Encoding::Cl100kBase).ok_or(EnterpriseError::MissingVocabulary)?;
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary)
            .map_err(|source| EnterpriseError::Encoding { source })?;
        Ok(Self {
            engine: Engine::Tiktoken(Arc::new(engine)),
        })
    }
}

impl TextTokenCounter for Tokenizer {
    fn count_response_tokens(&self, text: &str) -> u64 {
        self.engine
            .encode(text)
            .map_or(0, |ids| u64::try_from(ids.len()).unwrap_or_default())
    }
}

impl TextEncoding for Tokenizer {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        self.engine.encode(text).map(|ids| ids.len())
    }
}
