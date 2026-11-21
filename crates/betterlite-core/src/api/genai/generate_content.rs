mod adapter;
mod adapter_input;
mod answered;
pub mod async_stream;
pub mod async_unary;
mod caller;
mod dispatch;
pub mod error;
mod fields;
mod native;
pub mod stream;
mod tests;
pub mod unary;

use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::genai::chat_call::GenerateContentOutcome as AdapterOutcome;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::{
    AsyncGenerateContentOutcome as AsyncNativeOutcome, ClientSource,
    GenerateContentOutcome as NativeOutcome,
};
use crate::api::genai_http::stream::StreamSettings;
use crate::api::vector_store_http_write::service::SuppliedClient;
use crate::genai_setup::config::GenerateContentProviderConfigs;
use crate::schema::SchemaLimits;

const DEFAULT_MOCK_RESPONSE: &str = "This is a mock response from Google GenAI generate_content.";

// one call, field for field with LiteLLM's named parameters and the **kwargs it forwards to the handler
pub struct GenerateContentRequest<'request> {
    pub model: &'request str,
    pub contents: &'request Json,
    pub config: Option<&'request Object>,
    pub tools: Option<&'request Json>,
    pub extra_headers: Option<&'request IndexMap<String, String>>,
    pub extra_query: Option<&'request Object>,
    pub extra_body: Option<&'request Object>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'request str>,
    pub kwargs: &'request Object,
}

// every collaborator the four entry points call into, wired once by the caller
pub struct GenerateContentDeps<'deps, C: GenerateContentHttpConfig, S: ClientSource, Executor> {
    pub configs: &'deps GenerateContentProviderConfigs<'deps, C>,
    pub resolution: &'deps ResolutionContext<'deps>,
    pub capabilities: &'deps CatalogCapabilities,
    pub limits: SchemaLimits,
    pub source: &'deps S,
    pub supplied_client: Option<SuppliedClient<'deps, S::Client>>,
    pub executor: &'deps Executor,
    pub stream: StreamSettings,
    pub request_timeout: RequestTimeout,
    pub redactor: &'deps Redactor,
}

// what an entry point answers: the caller's mock, the chat adapter's outcome, or the native handler's
pub enum GenerateContentAnswer<Response> {
    Mock(Json),
    Adapter(AdapterOutcome),
    Native(NativeOutcome<Response>),
    AsyncNative(AsyncNativeOutcome<Response>),
}

// GenerateContentHelper.mock_generate_content_response: the fixed dict LiteLLM answers a string mock_response with
#[must_use]
pub fn mock_generate_content_response(mock_response: Option<&str>) -> Json {
    let text = mock_response.unwrap_or(DEFAULT_MOCK_RESPONSE);
    let mut part = Object::new();
    part.insert("text".to_owned(), Json::Text(text.to_owned()));
    let mut content = Object::new();
    content.insert("parts".to_owned(), Json::List(vec![Json::Object(part)]));
    content.insert("role".to_owned(), Json::Text("model".to_owned()));
    let mut candidate = Object::new();
    candidate.insert("content".to_owned(), Json::Object(content));
    candidate.insert("finishReason".to_owned(), Json::Text("STOP".to_owned()));
    candidate.insert("index".to_owned(), Json::Number(Number::from(0_u32)));
    candidate.insert("safetyRatings".to_owned(), Json::List(Vec::new()));
    let mut usage = Object::new();
    usage.insert(
        "promptTokenCount".to_owned(),
        Json::Number(Number::from(10_u32)),
    );
    usage.insert(
        "candidatesTokenCount".to_owned(),
        Json::Number(Number::from(20_u32)),
    );
    usage.insert(
        "totalTokenCount".to_owned(),
        Json::Number(Number::from(30_u32)),
    );
    let mut answer = Object::new();
    answer.insert("text".to_owned(), Json::Text(text.to_owned()));
    answer.insert(
        "candidates".to_owned(),
        Json::List(vec![Json::Object(candidate)]),
    );
    answer.insert("usageMetadata".to_owned(), Json::Object(usage));
    Json::Object(answer)
}
