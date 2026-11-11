use std::io::{self, Read as _};
use std::time::Duration;

use reqwest::blocking::{Client, ClientBuilder};
use reqwest::redirect::Policy;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;

use super::error::AgentSetupError;
use super::prompt::Prompter;
use super::resolve_credential;
use crate::agent_setup_profiles::Target;
use crate::claude_settings::choices::StaticToken;
use crate::cli_context::CliContext;
use crate::pi::listing::fetch_model_listing;
use crate::pi::{ListedModel, ListingFailure, PiSyncError, ProxyGet, ProxyResponse};

pub const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
pub const ANTHROPIC_VERSION_VALUE: &str = "2023-06-01";
pub const GATEWAY_CLIENT_HEADER: &str = "x-gateway-client";
pub const CLAUDE_CODE_CLIENT: &str = "claude-code";
const LISTED_MODELS_SHOWN: usize = 20;
const MAX_LISTING_BYTES: usize = 16 * 1024 * 1024;

struct HttpGet {
    client: Client,
}

impl ProxyGet for HttpGet {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout: Duration,
    ) -> Result<ProxyResponse, io::Error> {
        let mut request = self.client.get(url).timeout(timeout);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.expose_secret());
        }
        let response = request.send().map_err(io::Error::other)?;
        let status = response.status().as_u16();
        let limit = u64::try_from(MAX_LISTING_BYTES).unwrap_or(u64::MAX);
        let mut body = Vec::new();
        response
            .take(limit.saturating_add(1))
            .read_to_end(&mut body)?;
        if body.len() > MAX_LISTING_BYTES {
            return Err(io::Error::other("the /v1/models response is too large"));
        }
        Ok(ProxyResponse { status, body })
    }
}

pub async fn fetch_listing(
    base_url: &str,
    credential: &StaticToken,
    target: Target,
) -> Result<Vec<ListedModel>, AgentSetupError> {
    let base = base_url.to_owned();
    let key = credential.token.clone();
    let extra: &'static [(&'static str, &'static str)] = match target {
        Target::Claude => &[
            (ANTHROPIC_VERSION_HEADER, ANTHROPIC_VERSION_VALUE),
            (GATEWAY_CLIENT_HEADER, CLAUDE_CODE_CLIENT),
        ],
        Target::Codex => &[],
    };
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || {
        let client = ClientBuilder::new()
            .redirect(Policy::none())
            .build()
            .map_err(|source| PiSyncError::Unreachable {
                source: io::Error::other(source),
            })?;
        fetch_model_listing(&base, &key, &HttpGet { client }, extra)
    });
    let Some(joined) = tasks.join_next().await else {
        return Err(AgentSetupError::ListingTaskStopped);
    };
    let fetched = joined.map_err(AgentSetupError::ListingTask)?;
    fetched.map_err(|reason| listing_error(base_url, target, reason))
}

fn listing_error(base_url: &str, target: Target, reason: PiSyncError) -> AgentSetupError {
    match reason.kind() {
        Some(ListingFailure::Rejected) => AgentSetupError::ListingRejected {
            status: reason.status().unwrap_or_default(),
            reason,
        },
        Some(ListingFailure::Unreachable) => AgentSetupError::ListingUnreachable {
            base_url: base_url.to_owned(),
            reason,
        },
        Some(ListingFailure::Empty) => AgentSetupError::ListingEmpty {
            agent: match target {
                Target::Claude => "Claude Code",
                Target::Codex => "Codex",
            },
            reason,
        },
        _ => AgentSetupError::ListingFailed {
            base_url: base_url.to_owned(),
            reason,
        },
    }
}

pub struct ListingRequest<'request> {
    pub context: &'request CliContext,
    pub base_url: &'request str,
    pub credential: StaticToken,
    pub target: Target,
    pub repair: bool,
    pub prompter: &'request mut dyn Prompter,
}

pub async fn connection_listing(
    request: &mut ListingRequest<'_>,
) -> Result<(StaticToken, Vec<ListedModel>), AgentSetupError> {
    match fetch_listing(request.base_url, &request.credential, request.target).await {
        Ok(models) => Ok((request.credential.clone(), models)),
        Err(error) => {
            if !request.repair || !matches!(&error, AgentSetupError::ListingRejected { .. }) {
                return Err(error);
            }
            let replacement = loop {
                let entered = request.prompter.prompt_secret("Replacement virtual key")?;
                if !entered.is_empty() {
                    break SecretString::from(entered);
                }
            };
            let refreshed = resolve_credential(request.context, Some(&replacement))?;
            let models = fetch_listing(request.base_url, &refreshed, request.target).await?;
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
