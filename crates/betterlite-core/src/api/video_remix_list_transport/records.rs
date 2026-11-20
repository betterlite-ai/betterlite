use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

pub type Params = IndexMap<String, Json>;
pub type Headers = IndexMap<String, String>;

// The part of LiteLLM's logging object a provider transform reads
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoLogging {
    pub model_call_details: Params,
}

// api_base, litellm_params and headers are the last three arguments of every video request transform
#[derive(Debug, Clone, Copy)]
pub struct VideoRequestContext<'request> {
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

#[derive(Debug)]
pub struct VideoReply<'reply> {
    pub raw_response: &'reply UpstreamResponse,
    pub logging_obj: &'reply mut VideoLogging,
    pub custom_llm_provider: Option<&'reply str>,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoEnvironment<'request> {
    pub headers: &'request Headers,
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub litellm_params: Option<&'request Params>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VideoListQuery<'request> {
    pub after: Option<&'request str>,
    pub limit: Option<i64>,
    pub order: Option<&'request str>,
    pub extra_query: Option<&'request Params>,
}
