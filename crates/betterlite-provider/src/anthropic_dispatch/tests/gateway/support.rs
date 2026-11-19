use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;
use url::Url;

use crate::anthropic_dispatch::Anthropic;
use crate::anthropic_dispatch::base::Surface;
use crate::anthropic_dispatch::gateway::runtime::AnthropicRuntime;
use crate::anthropic_dispatch::native::tests::support::wire;
use crate::operation::{ChatOperation, OperationError, OperationRequest, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Provider;

pub(super) const MODEL: &str = "claude-3-haiku-20240307";
pub(super) const TIMEOUT: Duration = Duration::from_secs(10);

const TEST_CONFIG: &str = "litellm_settings:\n  user_url_validation: false\n";
const TEST_KEY: &str = "sk-test";

pub(super) struct Harness {
    pub(super) base: String,
    pub(super) client: ProxiedClient,
    pub(super) deployment: Deployment,
    pub(super) server: JoinSet<Vec<wire::Recorded>>,
}

impl Harness {
    pub(super) async fn start(answers: Vec<wire::Answer>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the loopback listener");
        let base = format!(
            "http://{}",
            listener.local_addr().expect("the loopback address")
        );
        let mut server = JoinSet::new();
        server.spawn(wire::serve(listener, answers));
        let client = client();
        let configured = deployment(&client, &base, MODEL, TIMEOUT);
        Self {
            base,
            client,
            deployment: configured,
            server,
        }
    }

    pub(super) async fn requests(&mut self) -> Vec<wire::Recorded> {
        let joined = timeout(TIMEOUT, self.server.join_next())
            .await
            .expect("the loopback answers every expected request")
            .expect("the loopback task");
        joined.expect("the loopback serves every planned answer")
    }
}

pub(super) async fn upstream(
    harness: &Harness,
    operation: OperationRequest,
) -> Result<(PreparedRequest, Upstream), OperationError> {
    let provider = Provider::Anthropic(Anthropic {
        surface: Surface::Messages,
    });
    let prepared = provider
        .prepare(&operation, &harness.deployment, &catalog())
        .expect("the anthropic adapter prepares chat");
    let upstream = provider
        .execute(&harness.client, &prepared, &harness.deployment)
        .await?;
    Ok((prepared, upstream))
}

pub(super) async fn complete(
    harness: &Harness,
    operation: OperationRequest,
) -> Result<OperationResult, OperationError> {
    let (prepared, upstream) = upstream(harness, operation).await?;
    let provider = Provider::Anthropic(Anthropic {
        surface: Surface::Messages,
    });
    provider.decode(&prepared, upstream)
}

pub(super) fn chat(body: String) -> OperationRequest {
    OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from(MODEL),
        request: ChatRequest::parse(body.as_bytes()).expect("the test request"),
        control: ControlOptions::default(),
    }))
}

pub(super) fn stream(body: String) -> OperationRequest {
    OperationRequest::Stream(Box::new(ChatOperation {
        model: String::from(MODEL),
        request: ChatRequest::parse(body.as_bytes()).expect("the test request"),
        control: ControlOptions::default(),
    }))
}

pub(super) fn deployment(
    client: &ProxiedClient,
    base: &str,
    model: &str,
    timeout: Duration,
) -> Deployment {
    Deployment {
        model: String::from(model),
        api_base: String::from(base),
        endpoint: Url::parse(&format!("{base}/v1/messages")).expect("the endpoint"),
        api_key: Some(SecretString::from(TEST_KEY)),
        authorization: None,
        organization: None,
        timeout,
        max_retries: None,
        drop_params: None,
        provider_affinity_header: None,
        anthropic: Some(runtime(client)),
    }
}

pub(super) fn client() -> ProxiedClient {
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the client")
}

fn runtime(client: &ProxiedClient) -> Arc<AnthropicRuntime> {
    let lookup = BTreeMap::from([(
        String::from("LITELLM_LOCAL_ANTHROPIC_BETA_HEADERS"),
        String::from("true"),
    )]);
    let config = ConfigLoader { lookup }
        .load(TEST_CONFIG)
        .expect("the test config loads");
    Arc::new(
        AnthropicRuntime::new(&config, client, Arc::new(ModelCatalog::default()))
            .expect("the anthropic runtime"),
    )
}

pub(super) fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}
