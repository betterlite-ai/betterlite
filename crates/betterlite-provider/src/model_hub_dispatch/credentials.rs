use super::{CustomPromptTable, HubDispatchError};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::model_hub_credentials::Settings;
use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
#[derive(Debug, Clone, Default)]
pub struct Globals<'source> {
    pub api_key: Option<&'source SecretString>,
    pub huggingface_key: Option<&'source SecretString>,
    pub replicate_key: Option<&'source SecretString>,
    pub predibase_key: Option<&'source SecretString>,
    pub predibase_tenant_id: Option<&'source SecretString>,
    pub api_base: Option<&'source str>,
    pub headers: Option<&'source Headers>,
    pub custom_prompt_dict: Option<&'source CustomPromptTable>,
}
impl<'source> Globals<'source> {
    #[must_use]
    pub fn from_settings(settings: &'source Settings) -> Self {
        Self {
            api_key: None,
            huggingface_key: settings.huggingface_key.as_ref(),
            replicate_key: settings.replicate_key.as_ref(),
            predibase_key: settings.predibase_key.as_ref(),
            predibase_tenant_id: settings.predibase_tenant_id.as_ref(),
            api_base: None,
            headers: None,
            custom_prompt_dict: None,
        }
    }
}
// one link of a main.py `a or b or c` chain, tried in order until one gives a value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Call,
    Pop(&'static str),
    ApiKey,
    HuggingfaceKey,
    ReplicateKey,
    PredibaseKey,
    PredibaseTenantId,
    ApiBase,
    // get_secret: an empty variable counts as unset
    Env(&'static str),
    // `name in os.environ`: an empty variable still counts
    EnvKeepEmpty(&'static str),
}
pub const HUGGINGFACE_CHAT_KEY: &[Step] = &[
    Step::Call,
    Step::HuggingfaceKey,
    Step::Env("HF_TOKEN"),
    Step::Env("HUGGINGFACE_API_KEY"),
    Step::ApiKey,
];
pub const HUGGINGFACE_EMBEDDING_KEY: &[Step] = &[
    Step::Call,
    Step::HuggingfaceKey,
    Step::Env("HUGGINGFACE_API_KEY"),
    Step::ApiKey,
];
pub const HUGGINGFACE_HUB_TOKEN: &[Step] = &[Step::Env("HUGGINGFACE_API_KEY")];
pub const HUGGINGFACE_RERANK_KEY: &[Step] = &[
    Step::Call,
    Step::HuggingfaceKey,
    Step::Env("HUGGINGFACE_API_KEY"),
];
pub const HUGGINGFACE_RERANK_BASE: &[Step] = &[
    Step::Call,
    Step::EnvKeepEmpty("HF_API_BASE"),
    Step::EnvKeepEmpty("HUGGINGFACE_API_BASE"),
];
pub const REPLICATE_KEY: &[Step] = &[
    Step::Call,
    Step::ReplicateKey,
    Step::ApiKey,
    Step::Env("REPLICATE_API_KEY"),
    Step::Env("REPLICATE_API_TOKEN"),
];
pub const PREDIBASE_TENANT_ID: &[Step] = &[
    Step::Pop("tenant_id"),
    Step::Pop("predibase_tenant_id"),
    Step::PredibaseTenantId,
    Step::Env("PREDIBASE_TENANT_ID"),
];
pub const PREDIBASE_BASE: &[Step] = &[
    Step::Call,
    Step::Pop("api_base"),
    Step::Pop("base_url"),
    Step::ApiBase,
    Step::Env("PREDIBASE_API_BASE"),
];
pub const PREDIBASE_KEY: &[Step] = &[
    Step::Call,
    Step::ApiKey,
    Step::PredibaseKey,
    Step::Env("PREDIBASE_API_KEY"),
];
pub struct Sources<'source> {
    pub call: Option<&'source str>,
    pub globals: &'source Globals<'source>,
    pub env: &'source dyn EnvironmentLookup,
}
fn pop_text(
    object: &mut Object,
    name: &'static str,
) -> Result<Option<SecretString>, HubDispatchError> {
    match object.shift_remove(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(SecretString::from(text))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(HubDispatchError::FieldType { field: name })
        }
    }
}
pub fn resolve(
    chain: &[Step],
    sources: &Sources<'_>,
    options: &mut Object,
) -> Result<Option<SecretString>, HubDispatchError> {
    for step in chain {
        let found = match *step {
            Step::Call => sources.call.map(SecretString::from),
            Step::Pop(name) => pop_text(options, name)?,
            Step::ApiKey => sources.globals.api_key.cloned(),
            Step::HuggingfaceKey => sources.globals.huggingface_key.cloned(),
            Step::ReplicateKey => sources.globals.replicate_key.cloned(),
            Step::PredibaseKey => sources.globals.predibase_key.cloned(),
            Step::PredibaseTenantId => sources.globals.predibase_tenant_id.cloned(),
            Step::ApiBase => sources.globals.api_base.map(SecretString::from),
            Step::Env(name) => sources
                .env
                .lookup(name)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            Step::EnvKeepEmpty(name) => sources.env.lookup(name).map(SecretString::from),
        };
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}
#[must_use]
pub fn huggingface_env(env: &dyn EnvironmentLookup) -> HuggingfaceEnv {
    HuggingfaceEnv {
        hf_api_base: env.lookup("HF_API_BASE"),
        huggingface_api_base: env.lookup("HUGGINGFACE_API_BASE"),
    }
}
