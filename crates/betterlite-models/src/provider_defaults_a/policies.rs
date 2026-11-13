use std::collections::BTreeSet;

use secrecy::SecretString;

#[derive(Debug, Clone, Default)]
pub struct ModelSets {
    pub openai_chat: BTreeSet<String>,
    pub openai_text_completion: BTreeSet<String>,
    pub ai21_chat: BTreeSet<String>,
}

// B2207 resolves these through its Mantle input policy; the defaults table only substitutes them.
#[derive(Debug, Clone)]
pub struct MantleEndpoint {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub base_model: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderPolicies {
    pub models: ModelSets,
    pub mantle: Option<MantleEndpoint>,
}
