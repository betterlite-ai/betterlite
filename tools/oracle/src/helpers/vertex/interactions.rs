use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::studio_interactions::urls::Lifecycle;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::config::{InteractionCall, VertexInteractionsConfig};
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

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
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    api_base: String,
    stream: bool,
    params: BTreeMap<String, String>,
    interaction_id: String,
    headers: IndexMap<String, String>,
    #[serde(default)]
    action: Option<Lifecycle>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Url,
    Lifecycle,
    Headers,
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
        let resolved = self.project.clone().or_else(|| project.map(str::to_owned));
        let token = SecretString::from(self.token.clone());
        Box::pin(async move {
            Ok(AccessToken {
                token,
                project: resolved.unwrap_or_default(),
            })
        })
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

fn interaction_call<'call>(
    call: &'call Call,
    settings: &'call VertexSettings,
) -> InteractionCall<'call> {
    InteractionCall {
        api_base: Some(call.api_base.as_str()),
        params: VertexParams {
            project: call.params.get("vertex_project").map(String::as_str),
            ai_project: call.params.get("vertex_ai_project").map(String::as_str),
            location: call.params.get("vertex_location").map(String::as_str),
            ai_location: call.params.get("vertex_ai_location").map(String::as_str),
            credentials: None,
            ai_credentials: None,
        },
        settings,
    }
}

fn rendered_headers(headers: &IndexMap<String, SecretString>) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

async fn resolve(
    config: &VertexInteractionsConfig,
    settings: &VertexSettings,
    call: &Call,
) -> crate::helpers::HelperResult {
    let given = interaction_call(call, settings);
    match call.function {
        Function::Url => {
            let url = config
                .get_complete_url(&given, call.stream)
                .await
                .or_value_error()?;
            Ok(json!({ "url": url }))
        }
        Function::Lifecycle => {
            let action = call
                .action
                .ok_or("a lifecycle call needs an action")
                .or_value_error()?;
            let request = config
                .lifecycle_request(&given, action, &call.interaction_id)
                .await
                .or_value_error()?;
            Ok(json!({ "url": request.url, "body": request.body }))
        }
        Function::Headers => {
            let headers = config
                .headers(&given, &call.headers)
                .await
                .or_value_error()?;
            Ok(json!({ "headers": rendered_headers(&headers) }))
        }
    }
}

mod tests;
