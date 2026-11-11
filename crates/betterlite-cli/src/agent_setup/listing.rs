use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use tokio::task::JoinSet;

use super::error::AgentSetupError;
use super::prompt::Prompter;
use super::resolve_credential;
use crate::agent_setup_profiles::Target;
use crate::claude_settings::choices::StaticToken;
use crate::cli_context::CliContext;
use crate::pi::listing::fetch_model_listing;
use crate::pi::{ListedModel, PiSyncError, ProxyGet, ProxyResponse};

pub const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
pub const ANTHROPIC_VERSION_VALUE: &str = "2023-06-01";
pub const GATEWAY_CLIENT_HEADER: &str = "x-gateway-client";
pub const CLAUDE_CODE_CLIENT: &str = "claude-code";
const LISTED_MODELS_SHOWN: usize = 20;
pub(crate) const MAX_LISTING_BYTES: usize = 16 * 1024 * 1024;
const PROXY_VARIABLES: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

struct HttpGet {
    client: ProxiedClient,
}

impl ProxyGet for HttpGet {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout: Duration,
    ) -> Result<ProxyResponse, io::Error> {
        let mut request = self.client.client.get(url).timeout(timeout);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.expose_secret());
        }
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(io::Error::other)?;
        runtime.block_on(async {
            let mut response = request.send().await.map_err(io::Error::other)?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
                append_chunk(&mut body, &chunk)?;
            }
            Ok(ProxyResponse { status, body })
        })
    }
}

fn append_chunk(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), io::Error> {
    if body.len().saturating_add(chunk.len()) > MAX_LISTING_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the /v1/models response is too large",
        ));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn listing_client(variables: &BTreeMap<String, String>) -> Result<ProxiedClient, PiSyncError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), variables).map_err(|source| {
        PiSyncError::Unreachable {
            source: io::Error::other(source),
        }
    })?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|source| PiSyncError::Unreachable {
        source: io::Error::other(source),
    })
}

fn proxy_environment(environment: &dyn EnvironmentLookup) -> BTreeMap<String, String> {
    let mut variables = BTreeMap::new();
    for name in PROXY_VARIABLES {
        if let Some(value) = environment.lookup(name) {
            variables.insert(name.to_owned(), value);
        }
    }
    variables
}

pub async fn fetch_listing(
    base_url: &str,
    credential: &StaticToken,
    target: Target,
    environment: &(dyn EnvironmentLookup + Send + Sync),
) -> Result<Vec<ListedModel>, AgentSetupError> {
    let base = base_url.to_owned();
    let key = credential.token.clone();
    let variables = proxy_environment(environment);
    let extra: &'static [(&'static str, &'static str)] = match target {
        Target::Claude => &[
            (ANTHROPIC_VERSION_HEADER, ANTHROPIC_VERSION_VALUE),
            (GATEWAY_CLIENT_HEADER, CLAUDE_CODE_CLIENT),
        ],
        Target::Codex => &[],
    };
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || {
        let client = listing_client(&variables)?;
        fetch_model_listing(&base, &key, &HttpGet { client }, extra)
    });
    let Some(joined) = tasks.join_next().await else {
        return Err(AgentSetupError::ListingTaskStopped);
    };
    let fetched = joined.map_err(AgentSetupError::ListingTask)?;
    fetched.map_err(|reason| {
        if let PiSyncError::Unreachable { source } = &reason
            && source.kind() == io::ErrorKind::InvalidData
        {
            return AgentSetupError::ListingTooLarge {
                base_url: base_url.to_owned(),
            };
        }
        listing_error(base_url, target, reason)
    })
}

fn listing_error(base_url: &str, target: Target, reason: PiSyncError) -> AgentSetupError {
    match reason {
        failure @ PiSyncError::ListingStatus {
            status: status @ (401 | 403),
        } => AgentSetupError::ListingRejected {
            status,
            reason: failure,
        },
        failure @ PiSyncError::Unreachable { .. } => AgentSetupError::ListingUnreachable {
            base_url: base_url.to_owned(),
            reason: failure,
        },
        failure @ (PiSyncError::NoModels | PiSyncError::NothingToRun) => {
            AgentSetupError::ListingEmpty {
                agent: match target {
                    Target::Claude => "Claude Code",
                    Target::Codex => "Codex",
                },
                reason: failure,
            }
        }
        failure @ (PiSyncError::ListingStatus { .. }
        | PiSyncError::ListingJson { .. }
        | PiSyncError::ListingInvalid { .. }
        | PiSyncError::ReadModels { .. }
        | PiSyncError::ParseModels { .. }
        | PiSyncError::Encode { .. }
        | PiSyncError::WriteModels { .. }
        | PiSyncError::NoHome) => AgentSetupError::ListingFailed {
            base_url: base_url.to_owned(),
            reason: failure,
        },
    }
}

pub struct ListingRequest<'request> {
    pub context: &'request CliContext,
    pub base_url: &'request str,
    pub credential: StaticToken,
    pub target: Target,
    pub repair: bool,
    pub environment: &'request (dyn EnvironmentLookup + Send + Sync),
    pub prompter: &'request mut dyn Prompter,
}

pub async fn connection_listing(
    request: &mut ListingRequest<'_>,
) -> Result<(StaticToken, Vec<ListedModel>), AgentSetupError> {
    let first = fetch_listing(
        request.base_url,
        &request.credential,
        request.target,
        request.environment,
    )
    .await;
    match first {
        Ok(models) => Ok((request.credential.clone(), models)),
        Err(error) => {
            if !request.repair || !matches!(&error, AgentSetupError::ListingRejected { .. }) {
                return Err(error);
            }
            let replacement = loop {
                let entered = request.prompter.prompt_secret("Replacement virtual key")?;
                if !entered.expose_secret().is_empty() {
                    break entered;
                }
            };
            let refreshed = resolve_credential(request.context, Some(&replacement))?;
            let models = fetch_listing(
                request.base_url,
                &refreshed,
                request.target,
                request.environment,
            )
            .await?;
            Ok((refreshed, models))
        }
    }
}

#[must_use]
pub fn starting_model<'listing>(
    name: &str,
    listing: &'listing [ListedModel],
) -> Option<&'listing str> {
    let by_source = listing
        .iter()
        .find(|listed| listed.source_model.as_deref() == Some(name));
    by_source.map_or_else(
        || {
            listing
                .iter()
                .find(|listed| listed.id == name)
                .map(|listed| listed.id.as_str())
        },
        |listed| Some(listed.id.as_str()),
    )
}

pub fn validated_model<'listing>(
    model: Option<&str>,
    listing: &'listing [ListedModel],
    base_url: &str,
) -> Result<Option<&'listing str>, AgentSetupError> {
    let Some(name) = model else {
        return Ok(None);
    };
    let Some(starting) = starting_model(name, listing) else {
        let shown = listing
            .iter()
            .take(LISTED_MODELS_SHOWN)
            .map(|listed| listed.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AgentSetupError::ModelNotServed {
            model: name.to_owned(),
            base_url: base_url.to_owned(),
            listed: shown,
        });
    };
    Ok(Some(starting))
}
