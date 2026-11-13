use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use reqwest::blocking::Client;

use crate::case::Request;
use crate::cli::Options;
use crate::gateway::{self, GatewayInput};
use crate::http::{self, Response};
use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::run::{Peer, PeerFactory, PeerStart, ProviderSplit};
use crate::mock::{MockProvider, ProviderRequest};
use crate::process::Gateway;

pub(super) struct GatewayPeerFactory<'input> {
    pub(super) options: &'input Options,
    pub(super) group_dir: PathBuf,
    pub(super) provider: Arc<MockProvider>,
    pub(super) client: Client,
}

impl PeerFactory for GatewayPeerFactory<'_> {
    fn start(&mut self, input: PeerStart<'_>) -> Result<Box<dyn Peer>, ScenarioError> {
        let config_path = self
            .group_dir
            .join(format!("scenario-{}-config.yaml", input.role.label()));
        fs::write(&config_path, input.config).map_err(|source| ScenarioError::Artifact {
            path: config_path.clone(),
            source,
        })?;
        let gateway_input = GatewayInput {
            options: self.options,
            group_dir: &self.group_dir,
            config_path: &config_path,
            case_env: input.env,
        };
        let gateway = gateway::start(&gateway_input, input.role.is_first())
            .map_err(|source| ScenarioError::Request(Box::new(source)))?;
        Ok(Box::new(GatewayPeer {
            gateway,
            client: self.client.clone(),
            provider: Arc::clone(&self.provider),
        }))
    }
}

struct GatewayPeer {
    gateway: Gateway,
    client: Client,
    provider: Arc<MockProvider>,
}

impl Peer for GatewayPeer {
    fn send(&mut self, request: &Request) -> Result<Response, ScenarioError> {
        http::send(&self.client, &self.gateway.base_url, request)
            .map_err(|source| ScenarioError::Request(Box::new(source)))
    }

    fn provider_requests(&self) -> Result<Vec<ProviderRequest>, ScenarioError> {
        Ok(Vec::new())
    }

    fn provider_splits(
        &mut self,
        counts: &[usize],
    ) -> Result<Option<Vec<ProviderSplit>>, ScenarioError> {
        let calls = self
            .provider
            .finish_run()
            .map_err(|source| ScenarioError::Request(Box::new(source)))?;
        let trace = calls
            .into_iter()
            .map(|call| call.request)
            .collect::<Vec<ProviderRequest>>();
        let mut splits = Vec::with_capacity(counts.len());
        let mut previous = 0_usize;
        for (index, count) in counts.iter().enumerate() {
            let last = index.saturating_add(1) == counts.len();
            let end = if last {
                trace.len()
            } else {
                previous.saturating_add(*count).min(trace.len())
            };
            splits.push(ProviderSplit {
                requests: trace.get(previous..end).unwrap_or_default().to_vec(),
                total: end,
            });
            previous = end;
        }
        Ok(Some(splits))
    }

    fn log(&self) -> Result<String, ScenarioError> {
        super::adapters::read_log(&self.gateway.log_path)
    }

    fn stop(&mut self) -> Result<(), ScenarioError> {
        self.gateway
            .stop()
            .map_err(|source| ScenarioError::Request(Box::new(source)))
    }
}
