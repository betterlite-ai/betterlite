use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::api_base_lookup::{
    ApiBaseContext, ApiBaseError, ApiBaseHooks, ApiBaseParams, LoginFreeProvider, get_api_base,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    #[serde(default)]
    params: Params,
    #[serde(default)]
    model_alias_map: BTreeMap<String, String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    hooks: Hooks,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {
    api_base: Option<String>,
    base_url: Option<String>,
    custom_llm_provider: Option<String>,
    api_key: Option<String>,
    vertex_location: Option<String>,
    vertex_project: Option<String>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hooks {
    github_copilot: Option<String>,
    chatgpt: Option<String>,
}

struct FakeHooks<'hooks> {
    hooks: &'hooks Hooks,
}

impl ApiBaseHooks for FakeHooks<'_> {
    fn login_free_api_base(&self, provider: LoginFreeProvider) -> Option<String> {
        match provider {
            LoginFreeProvider::GithubCopilot => self.hooks.github_copilot.clone(),
            LoginFreeProvider::Chatgpt => self.hooks.chatgpt.clone(),
        }
    }

    // Stands in for B1304's create_vertex_url with the claude partner, which B8634 wires into the hook.
    fn vertex_claude_url(
        &self,
        location: &str,
        project: &str,
        model: &str,
        stream: bool,
    ) -> Result<String, ApiBaseError> {
        let mut characters = location.chars();
        let first_ok = characters
            .next()
            .is_some_and(|first| first.is_ascii_lowercase());
        let rest_ok =
            characters.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
        if location != "global" && !(first_ok && rest_ok) {
            return Err(ApiBaseError::InvalidVertexLocation {
                message: String::from("Invalid vertex_location format"),
            });
        }
        let base = if location == "global" {
            String::from("https://aiplatform.googleapis.com")
        } else if location.contains('-') {
            format!("https://{location}-aiplatform.googleapis.com")
        } else {
            format!("https://aiplatform.{location}.rep.googleapis.com")
        };
        let action = if stream {
            "streamRawPredict"
        } else {
            "rawPredict"
        };
        Ok(format!(
            "{base}/v1/projects/{project}/locations/{location}/publishers/anthropic/models/{model}:{action}"
        ))
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let catalog = ModelCatalog::default();
        let known = BTreeSet::new();
        let custom = BTreeSet::new();
        let resolution = ResolutionContext {
            catalog: &catalog,
            known_model_names: &known,
            custom_providers: &custom,
            environment: &input.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let hooks = FakeHooks {
            hooks: &input.hooks,
        };
        let context = ApiBaseContext {
            model_alias_map: &input.model_alias_map,
            hooks: &hooks,
            resolution: &resolution,
        };
        let params = ApiBaseParams {
            api_base: input.params.api_base.clone(),
            base_url: input.params.base_url.clone(),
            custom_llm_provider: input.params.custom_llm_provider.clone(),
            api_key: input.params.api_key.clone().map(SecretString::from),
            vertex_location: input.params.vertex_location.clone(),
            vertex_project: input.params.vertex_project.clone(),
            stream: input.params.stream,
        };
        let api_base = get_api_base(&input.model, &params, &context)
            .map_err(|error| api_base_error(&error))?;
        Ok(json!({ "api_base": api_base }))
    }
}

fn api_base_error(error: &ApiBaseError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ApiBaseError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
