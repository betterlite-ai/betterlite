use super::{CustomPromptTable, HubDispatchError};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::model_hub_credentials::Settings;
use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
pub const REPLICATE_API_BASE: &str = "https://api.replicate.com/v1";
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
fn environment_secret(env: &dyn EnvironmentLookup, name: &str) -> Option<SecretString> {
    env.lookup(name)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}
fn environment_text(env: &dyn EnvironmentLookup, name: &str) -> Option<String> {
    env.lookup(name).filter(|value| !value.is_empty())
}
#[must_use]
pub fn huggingface_chat_key(
    api_key: Option<&SecretString>,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    api_key
        .cloned()
        .or_else(|| globals.huggingface_key.cloned())
        .or_else(|| environment_secret(env, "HF_TOKEN"))
        .or_else(|| environment_secret(env, "HUGGINGFACE_API_KEY"))
        .or_else(|| globals.api_key.cloned())
}
#[must_use]
pub fn huggingface_embedding_key(
    api_key: Option<&SecretString>,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    api_key
        .cloned()
        .or_else(|| globals.huggingface_key.cloned())
        .or_else(|| environment_secret(env, "HUGGINGFACE_API_KEY"))
        .or_else(|| globals.api_key.cloned())
}
#[must_use]
pub fn huggingface_hub_token(env: &dyn EnvironmentLookup) -> Option<SecretString> {
    environment_secret(env, "HUGGINGFACE_API_KEY")
}
#[must_use]
pub fn huggingface_env(env: &dyn EnvironmentLookup) -> HuggingfaceEnv {
    HuggingfaceEnv {
        hf_api_base: env.lookup("HF_API_BASE"),
        huggingface_api_base: env.lookup("HUGGINGFACE_API_BASE"),
    }
}
#[must_use]
pub fn huggingface_rerank_key(
    api_key: Option<&SecretString>,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    api_key
        .cloned()
        .or_else(|| globals.huggingface_key.cloned())
        .or_else(|| environment_secret(env, "HUGGINGFACE_API_KEY"))
}
#[must_use]
pub fn huggingface_rerank_base(
    api_base: Option<&str>,
    env: &dyn EnvironmentLookup,
) -> Option<String> {
    api_base
        .map(str::to_owned)
        .or_else(|| env.lookup("HF_API_BASE"))
        .or_else(|| env.lookup("HUGGINGFACE_API_BASE"))
}
#[must_use]
pub fn replicate_key(
    api_key: Option<&SecretString>,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    api_key
        .cloned()
        .or_else(|| globals.replicate_key.cloned())
        .or_else(|| globals.api_key.cloned())
        .or_else(|| environment_secret(env, "REPLICATE_API_KEY"))
        .or_else(|| environment_secret(env, "REPLICATE_API_TOKEN"))
}
#[must_use]
pub fn replicate_base(
    api_base: Option<&str>,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
) -> String {
    api_base
        .map(str::to_owned)
        .or_else(|| globals.api_base.map(str::to_owned))
        .or_else(|| environment_text(env, "REPLICATE_API_BASE"))
        .unwrap_or_else(|| REPLICATE_API_BASE.to_owned())
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
pub fn predibase_tenant_id(
    optional_params: &mut Object,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
) -> Result<Option<SecretString>, HubDispatchError> {
    if let Some(tenant) = pop_text(optional_params, "tenant_id")? {
        return Ok(Some(tenant));
    }
    if let Some(tenant) = pop_text(optional_params, "predibase_tenant_id")? {
        return Ok(Some(tenant));
    }
    Ok(globals
        .predibase_tenant_id
        .cloned()
        .or_else(|| environment_secret(env, "PREDIBASE_TENANT_ID")))
}
pub fn predibase_base(
    api_base: Option<&str>,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
    optional_params: &mut Object,
) -> Result<Option<String>, HubDispatchError> {
    if let Some(base) = api_base {
        return Ok(Some(base.to_owned()));
    }
    if let Some(base) = pop_text(optional_params, "api_base")? {
        return Ok(Some(base.expose_secret().to_owned()));
    }
    if let Some(base) = pop_text(optional_params, "base_url")? {
        return Ok(Some(base.expose_secret().to_owned()));
    }
    Ok(globals
        .api_base
        .map(str::to_owned)
        .or_else(|| environment_text(env, "PREDIBASE_API_BASE")))
}
#[must_use]
pub fn predibase_key(
    api_key: Option<&SecretString>,
    globals: &Globals<'_>,
    env: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    api_key
        .cloned()
        .or_else(|| globals.api_key.cloned())
        .or_else(|| globals.predibase_key.cloned())
        .or_else(|| environment_secret(env, "PREDIBASE_API_KEY"))
}
