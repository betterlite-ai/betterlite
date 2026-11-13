use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::studio_interactions::urls::Lifecycle;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::config::{InteractionCall, VertexInteractionsConfig};
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use betterlite_vertex::vertex_interactions::{API_VERSION, DEFAULT_LOCATION, PROVIDER};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mint: Mint,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mint {
    token: String,
    project: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Url(Box<UrlCall>),
    Lifecycle(LifecycleCall),
    Headers(HeadersCall),
    Identity,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    #[serde(rename = "vertex_project")]
    project: Option<String>,
    #[serde(rename = "vertex_ai_project")]
    ai_project: Option<String>,
    #[serde(rename = "vertex_location")]
    location: Option<String>,
    #[serde(rename = "vertex_ai_location")]
    ai_location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleCall {
    action: Lifecycle,
    interaction_id: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadersCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    litellm_params: Params,
}

// the injected mint of the source: a fixed token, and the case's project or the one the getters resolved
struct FixtureMint {
    token: String,
    project: Option<String>,
}

impl MintAccessToken for FixtureMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        let resolved = self
            .project
            .clone()
            .or_else(|| project.map(str::to_owned))
            .unwrap_or_default();
        let token = SecretString::from(self.token.clone());
        Box::pin(std::future::ready(Ok(AccessToken {
            token,
            project: resolved,
        })))
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .or_value_error()?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> crate::helpers::HelperResult {
    let settings = VertexSettings::default();
    let config = VertexInteractionsConfig {
        mint: Arc::new(FixtureMint {
            token: input.mint.token.clone(),
            project: input.mint.project.clone(),
        }),
    };
    let mut replies = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        replies.push(resolve(&config, &settings, call).await?);
    }
    Ok(json!({ "calls": replies }))
}

async fn resolve(
    config: &VertexInteractionsConfig,
    settings: &VertexSettings,
    call: &Call,
) -> crate::helpers::HelperResult {
    match call {
        Call::Url(given) => url_call(config, settings, given).await.or_value_error(),
        Call::Lifecycle(given) => lifecycle_call(config, settings, given)
            .await
            .or_value_error(),
        Call::Headers(given) => headers_call(config, settings, given).await.or_value_error(),
        Call::Identity => Ok(json!({
            "provider": PROVIDER,
            "api_version": API_VERSION,
            "default_location": DEFAULT_LOCATION,
        })),
    }
}

async fn url_call(
    config: &VertexInteractionsConfig,
    settings: &VertexSettings,
    given: &UrlCall,
) -> Result<Value, betterlite_vertex::vertex_interactions::error::VertexInteractionsError> {
    let url = config
        .get_complete_url(
            &interaction_call(given.api_base.as_deref(), &given.litellm_params, settings),
            given.stream,
        )
        .await?;
    Ok(json!({ "url": url }))
}

async fn lifecycle_call(
    config: &VertexInteractionsConfig,
    settings: &VertexSettings,
    given: &LifecycleCall,
) -> Result<Value, betterlite_vertex::vertex_interactions::error::VertexInteractionsError> {
    let call = interaction_call(given.api_base.as_deref(), &given.litellm_params, settings);
    let request = config
        .lifecycle_request(&call, given.action, &given.interaction_id)
        .await?;
    Ok(json!({ "url": request.url, "body": request.body }))
}

async fn headers_call(
    config: &VertexInteractionsConfig,
    settings: &VertexSettings,
    given: &HeadersCall,
) -> Result<Value, betterlite_vertex::vertex_interactions::error::VertexInteractionsError> {
    let caller: IndexMap<String, String> = given
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let call = interaction_call(None, &given.litellm_params, settings);
    let headers = config.headers(&call, &caller).await?;
    let rendered: IndexMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect();
    Ok(json!({ "headers": rendered }))
}

fn interaction_call<'call>(
    api_base: Option<&'call str>,
    params: &'call Params,
    settings: &'call VertexSettings,
) -> InteractionCall<'call> {
    InteractionCall {
        api_base,
        params: VertexParams {
            project: params.project.as_deref(),
            ai_project: params.ai_project.as_deref(),
            location: params.location.as_deref(),
            ai_location: params.ai_location.as_deref(),
            credentials: None,
            ai_credentials: None,
        },
        settings,
    }
}

mod tests;
