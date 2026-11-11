use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::identity::declared::authenticating_provider;
use crate::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, ResolvedModel, resolve,
};

mod login_free;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiBaseError {
    #[error("{message}")]
    InvalidVertexLocation { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFreeProvider {
    GithubCopilot,
    Chatgpt,
}

pub trait ApiBaseHooks {
    fn login_free_api_base(&self, provider: LoginFreeProvider) -> Option<String>;
    fn vertex_claude_url(
        &self,
        location: &str,
        project: &str,
        model: &str,
        stream: bool,
    ) -> Result<String, ApiBaseError>;
}

#[derive(Debug, Clone, Default)]
pub struct ApiBaseParams {
    pub api_base: Option<String>,
    pub base_url: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub api_key: Option<SecretString>,
    pub vertex_location: Option<String>,
    pub vertex_project: Option<String>,
    pub stream: bool,
}

pub struct ApiBaseContext<'context> {
    pub model_alias_map: &'context BTreeMap<String, String>,
    pub hooks: &'context dyn ApiBaseHooks,
    pub resolution: &'context ResolutionContext<'context>,
}

// Ports get_api_base: params, alias map, declared login-free providers, resolver, vertex, defaults.
pub fn get_api_base(
    model: &str,
    params: &ApiBaseParams,
    context: &ApiBaseContext<'_>,
) -> Result<Option<String>, ApiBaseError> {
    if let Some(api_base) = &params.api_base {
        return Ok(Some(api_base.clone()));
    }
    if let Some(base_url) = &params.base_url
        && !base_url.is_empty()
    {
        return Ok(Some(base_url.clone()));
    }
    let mut selected_model = model.to_owned();
    if let Some(aliased) = context.model_alias_map.get(&selected_model) {
        selected_model = aliased.clone();
    }
    if let Some(declared) =
        authenticating_provider(Some(&selected_model), params.custom_llm_provider.as_deref())
    {
        return Ok(login_free::api_base(declared, context.hooks));
    }
    resolve_then_default(&selected_model, params, context)
}

fn resolve_then_default(
    model: &str,
    params: &ApiBaseParams,
    context: &ApiBaseContext<'_>,
) -> Result<Option<String>, ApiBaseError> {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: params.custom_llm_provider.as_deref(),
        api_base: params.api_base.as_deref(),
        api_key: params.api_key.as_ref(),
    };
    let (resolved_model, provider) = match resolve(&request, context.resolution) {
        Ok(ResolvedModel {
            model: resolved_model,
            provider,
            api_base,
            ..
        }) => match api_base {
            Some(dynamic_api_base) => return Ok(Some(dynamic_api_base)),
            None => (resolved_model, Some(provider)),
        },
        Err(error) => {
            tracing::debug!(error = %error, "resolving the provider for an api base failed");
            (model.to_owned(), None)
        }
    };
    if let (Some(location), Some(project)) = (&params.vertex_location, &params.vertex_project) {
        return vertex_api_base(
            &resolved_model,
            location,
            project,
            params.stream,
            context.hooks,
        );
    }
    Ok(provider_default_api_base(
        &resolved_model,
        provider.as_ref(),
        params.stream,
    ))
}

fn vertex_api_base(
    model: &str,
    location: &str,
    project: &str,
    stream: bool,
    hooks: &dyn ApiBaseHooks,
) -> Result<Option<String>, ApiBaseError> {
    if model.contains("claude") {
        return hooks
            .vertex_claude_url(location, project, model, stream)
            .map(Some);
    }
    let action = if stream {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    Ok(Some(format!(
        "{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:{action}"
    )))
}

fn provider_default_api_base(
    model: &str,
    provider: Option<&ProviderIdentity>,
    stream: bool,
) -> Option<String> {
    let action = if stream {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    match provider.map(ProviderIdentity::slug) {
        Some("gemini") => Some(format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:{action}"
        )),
        Some("openai") => Some(String::from("https://api.openai.com")),
        _ => None,
    }
}

mod tests;
