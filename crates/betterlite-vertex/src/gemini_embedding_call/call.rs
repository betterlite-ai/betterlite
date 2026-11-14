use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::decisions::Extra;
use secrecy::SecretString;

use crate::endpoint_target::target::TargetProvider;
use crate::gemini_embedding_records::input::EmbeddingInput;

// LiteLLM's default for the timeout argument
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    Sync,
    Async,
}

#[derive(Clone, Copy)]
pub struct EmbeddingCall<'call> {
    pub provider: TargetProvider,
    pub execution: Execution,
    pub model: &'call str,
    pub input: &'call EmbeddingInput,
    pub optional_params: &'call Extra,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub credentials: Option<&'call ConfiguredCredentials>,
    pub vertex_project: Option<&'call str>,
    pub vertex_location: Option<&'call str>,
    pub timeout: Option<Duration>,
    pub extra_headers: &'call BTreeMap<String, String>,
}
