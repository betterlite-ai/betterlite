use super::call::ReturnDocuments;
use super::selection::SelectedConfig;
use betterlite_error::dispatch::RawException;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::future::Future;
pub type Params = BTreeMap<String, Json>;
pub type Headers = BTreeMap<String, String>;
// What get_optional_rerank_params reads: the named arguments, drop_params and the remaining kwargs
pub struct OptionsRequest<'request> {
    pub config: SelectedConfig,
    pub model: &'request str,
    pub drop_params: bool,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    pub custom_llm_provider: Option<&'request str>,
    pub top_n: Option<i64>,
    pub rank_fields: Option<&'request [String]>,
    pub return_documents: ReturnDocuments,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<&'request str>,
    pub non_default_params: &'request mut Params,
}
// One provider call: the credentials the branch selected, the mapped options and the parameters LiteLLM forwards
pub struct SendRequest<'request> {
    pub config: SelectedConfig,
    pub provider: &'request str,
    pub model: &'request str,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub generic_params: &'request Params,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub headers: &'request Headers,
    pub timeout_seconds: Option<f64>,
    pub is_async: bool,
    pub top_n: Option<i64>,
    pub rank_fields: Option<&'request [String]>,
    pub return_documents: ReturnDocuments,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<&'request str>,
}
// The provider operations the dispatcher drives: B1013's mapping and B2454's transport, supplied by the caller
pub trait RerankOperations {
    fn map_options(&self, request: OptionsRequest<'_>) -> Result<Params, RawException>;
    fn send(
        &self,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<RerankResponse, RawException>>;
}
// A failure the dispatcher raises itself: str(e) carries the text and nothing overrides it, as a ValueError does
#[must_use]
pub fn value_failure(message: &str, model: &str, provider: &str) -> RawException {
    let mut failure = provider_failure(0, message, model, provider, "ValueError");
    failure.status_code = None;
    failure.message = None;
    failure
}
// A provider's exception: message is str(e) and the attribute exception_type() overrides error_str with
#[must_use]
pub fn provider_failure(
    status_code: u16,
    message: &str,
    model: &str,
    provider: &str,
    exception_type: &str,
) -> RawException {
    RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code: Some(status_code),
        status_code_is_synthesized: false,
        message: Some(message.to_owned()),
        body: None,
        exception_type: exception_type.to_owned(),
    }
}
