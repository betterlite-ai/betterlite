use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Mutex, PoisonError};

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::stream::{self, StreamExt as _};
use thiserror::Error;

use crate::api::genai::chat_call::{
    CompletionCall, CompletionExecutor, CompletionOutput, ForwardedFields, Operation,
};
use crate::api::genai_chat_input::TranslationContext;
use crate::api::genai_chat_input::input::{GenerateContentRequest, GenerateLiteLLMParams};
use crate::schema::SchemaLimits;

const REQUEST: &str = r#"{"model":"gpt-4o","contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;
const RESPONSE: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}"#;
const NO_CHOICES: &str =
    r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[]}"#;
const CHUNK: &str = r#"{"id":"chunk-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null}],"usage":null}"#;

pub(super) const TRANSLATED_RESPONSE: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP","index":0,"safetyRatings":[]}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":5,"totalTokenCount":8}}"#;
pub(super) const TRANSLATED_FRAME: &str = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"a\"}],\"role\":\"model\"},\"finishReason\":null,\"index\":0,\"safetyRatings\":[]}]}\n\n";

pub(super) struct Environment {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    variables: BTreeMap<String, String>,
    capabilities: CatalogCapabilities,
    limits: SchemaLimits,
}

impl Environment {
    pub(super) fn new() -> Self {
        Self::limited(SchemaLimits::default())
    }

    pub(super) fn limited(limits: SchemaLimits) -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            variables: BTreeMap::new(),
            capabilities: CatalogCapabilities {
                chat_models: BTreeSet::from([String::from("gpt-4o")]),
                text_models: BTreeSet::new(),
            },
            limits,
        }
    }

    pub(super) fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.variables,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    pub(super) fn context<'env>(
        &'env self,
        resolution: &'env ResolutionContext<'env>,
    ) -> TranslationContext<'env> {
        TranslationContext {
            resolution,
            model_capabilities: &self.capabilities,
            limits: self.limits,
        }
    }
}

pub(super) fn request() -> GenerateContentRequest {
    serde_json::from_str(REQUEST).expect("a GenAI request")
}

pub(super) fn litellm_params() -> GenerateLiteLLMParams {
    serde_json::from_str(r#"{"custom_llm_provider":"openai","max_retries":2}"#)
        .expect("LiteLLM params")
}

pub(super) fn forwarded_fields() -> ForwardedFields {
    ForwardedFields {
        metadata: Some(
            serde_json::from_str(r#"{"user_api_key":"k1","tags":["a"]}"#).expect("metadata"),
        ),
        extra_headers: Some(
            serde_json::from_str(r#"{"Editor-Version":"vscode/1.0"}"#).expect("headers"),
        ),
        proxy_server_request: Some(
            serde_json::from_str(r#"{"url":"http://proxy","method":"POST"}"#)
                .expect("proxy request"),
        ),
    }
}

pub(super) fn response_output() -> CompletionOutput {
    CompletionOutput::Response(Box::new(
        serde_json::from_str::<ChatResponse>(RESPONSE).expect("a chat response"),
    ))
}

pub(super) fn empty_response_output() -> CompletionOutput {
    CompletionOutput::Response(Box::new(
        serde_json::from_str::<ChatResponse>(NO_CHOICES).expect("a chat response"),
    ))
}

pub(super) fn stream_output() -> CompletionOutput {
    let chunk = serde_json::from_str::<Chunk>(CHUNK).expect("a stream chunk");
    CompletionOutput::Stream(stream::iter(vec![chunk]).boxed())
}

#[derive(Debug, Error)]
#[error("{0}")]
pub(super) struct ScriptedFailure(pub(super) &'static str);

pub(super) struct Scripted {
    answer: Mutex<Option<Result<CompletionOutput, ScriptedFailure>>>,
    seen: Mutex<Vec<(Operation, CompletionCall)>>,
}

impl Scripted {
    pub(super) fn answering(answer: Result<CompletionOutput, ScriptedFailure>) -> Self {
        Self {
            answer: Mutex::new(Some(answer)),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<(Operation, CompletionCall)> {
        self.seen.lock().expect("the seen calls").clone()
    }

    fn answer(
        &self,
        operation: Operation,
        call: CompletionCall,
    ) -> Result<CompletionOutput, ScriptedFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((operation, call));
        let mut scripted = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        scripted
            .take()
            .unwrap_or(Err(ScriptedFailure("the scripted answer was used twice")))
    }
}

impl CompletionExecutor for Scripted {
    type Error = ScriptedFailure;

    fn completion(&self, call: CompletionCall) -> Result<CompletionOutput, ScriptedFailure> {
        self.answer(Operation::Completion, call)
    }

    fn acompletion(
        &self,
        call: CompletionCall,
    ) -> impl Future<Output = Result<CompletionOutput, ScriptedFailure>> + Send {
        let answer = self.answer(Operation::Acompletion, call);
        async move { answer }
    }
}
