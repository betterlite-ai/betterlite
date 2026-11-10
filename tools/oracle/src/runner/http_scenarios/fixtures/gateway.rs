use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use reqwest::blocking::Client;

use super::scripted::ScriptedProvider;
use crate::case::Request;
use crate::cli::Options;
use crate::gateway::{self, GatewayInput};
use crate::http::{self, Response};
use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::run::{Peer, PeerFactory, PeerStart};
use crate::mock::ProviderRequest;
use crate::paths::free_port;
use crate::process::Gateway;

pub(super) struct GatewayPeerFactory<'input> {
    pub(super) options: &'input Options,
    pub(super) group_dir: PathBuf,
    pub(super) provider: Arc<ScriptedProvider>,
    pub(super) client: Client,
    pub(super) started: bool,
}

impl PeerFactory for GatewayPeerFactory<'_> {
    fn start(&mut self, input: PeerStart<'_>) -> Result<Box<dyn Peer>, ScenarioError> {
        if self.started {
            self.provider
                .reset()
                .map_err(|source| ScenarioError::Request(Box::new(source)))?;
        }
        self.started = true;
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
        let port = free_port().map_err(|source| ScenarioError::Request(Box::new(source)))?;
        let spec = gateway::spec(&gateway_input, input.role.is_first(), port)
            .map_err(|source| ScenarioError::Request(Box::new(source)))?;
        let mut gateway =
            Gateway::start(spec).map_err(|source| ScenarioError::Request(Box::new(source)))?;
        gateway
            .wait_ready(&self.client)
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
    provider: Arc<ScriptedProvider>,
}

impl Peer for GatewayPeer {
    fn send(&mut self, request: &Request) -> Result<Response, ScenarioError> {
        http::send(&self.client, &self.gateway.base_url, request)
            .map_err(|source| ScenarioError::Request(Box::new(source)))
    }

    fn provider_requests(&self) -> Result<Vec<ProviderRequest>, ScenarioError> {
        self.provider
            .requests()
            .map_err(|source| ScenarioError::Request(Box::new(source)))
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
