use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{ModelDeployment, ProxyTrust};
use betterlite_core::api::vector_store_contracts::executor::DirectEmbeddingExecutor;
use betterlite_core::api::vector_store_contracts::router::{
    RouterEmbeddingExecutor, VectorStoreRouter,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::api::vector_store_contracts::config::{
    Params, VectorStoreEmbeddingExecutor,
};
use betterlite_provider::api::vector_store_contracts::error::{
    BaseLlmException, VectorStoreEmbeddingError,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde::Serialize;
use tokio::runtime::Builder;
use tokio::time::timeout;

use super::VectorStoreEmbeddingCall;
use super::capture::{Execution, Stop};
use super::error::SdkHttpError;
use super::native::openai_defaults;
use super::result::{EmbeddingOutcomes, SdkResult};

const POLL: Duration = Duration::from_millis(10);
const TIMEOUT_SECONDS: u64 = 30;

#[derive(Serialize)]
struct Config<'call> {
    model_list: Vec<Entry<'call>>,
}

#[derive(Clone, Copy, Serialize)]
struct Entry<'call> {
    model_name: &'call str,
    litellm_params: EntryParams<'call>,
}

#[derive(Clone, Copy, Serialize)]
struct EntryParams<'call> {
    model: &'call str,
    api_base: &'call str,
    api_key: &'call str,
    timeout: u64,
}

pub(crate) fn execute(
    call: &VectorStoreEmbeddingCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
    stop: &Stop<'_>,
) -> Execution {
    let result = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(SdkHttpError::Io)
        .and_then(|runtime| runtime.block_on(send_until_stopped(call, api_base, env, stop)));
    Execution {
        result,
        log: Vec::new(),
    }
}

// the request future is dropped, and its connection with it, as soon as the stop fires
async fn send_until_stopped(
    call: &VectorStoreEmbeddingCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
    stop: &Stop<'_>,
) -> Result<SdkResult, SdkHttpError> {
    let mut sending = Box::pin(send(call, api_base, env));
    loop {
        stop.check()?;
        if let Ok(sent) = timeout(POLL, &mut sending).await {
            return sent;
        }
    }
}

async fn send(
    call: &VectorStoreEmbeddingCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    let (direct_entries, router_entries) = entries(call, api_base);
    let (_direct_config, direct_deployments) = deployments(direct_entries, env)?;
    let direct = DirectEmbeddingExecutor {
        executor: chat_executor(&client, direct_deployments.clone()),
        deployments: direct_deployments,
    };
    let outcomes = if call.router.is_some() {
        let (router_config, router_deployments) = deployments(router_entries, env)?;
        let handle = FakeRouter {
            direct: DirectEmbeddingExecutor {
                executor: chat_executor(&client, router_deployments.clone()),
                deployments: router_deployments,
            },
            deployments: router_config,
        };
        let executor = RouterEmbeddingExecutor {
            router: handle,
            metadata: call.metadata.clone(),
            direct,
        };
        both_modes(&executor, call).await
    } else {
        both_modes(&direct, call).await
    };
    let projected = outcomes.map_err(|source| SdkHttpError::Embedding {
        source: Box::new(source),
    })?;
    Ok(SdkResult::Embedding { value: projected })
}

async fn both_modes<E>(
    executor: &E,
    call: &VectorStoreEmbeddingCall,
) -> Result<EmbeddingOutcomes, VectorStoreEmbeddingError>
where
    E: VectorStoreEmbeddingExecutor,
{
    let sync = executor
        .embed(&call.model, &call.query, &call.configuration)
        .await?;
    let asynchronous = executor
        .aembed(&call.model, &call.query, &call.configuration)
        .await?;
    Ok(EmbeddingOutcomes { sync, asynchronous })
}

// the harness owns api_base, so every deployment the case names points at the mock
fn entry<'call>(
    model_name: &'call str,
    model: &'call str,
    call: &'call VectorStoreEmbeddingCall,
    api_base: &'call str,
) -> Entry<'call> {
    Entry {
        model_name,
        litellm_params: EntryParams {
            model,
            api_base,
            api_key: &call.api_key,
            timeout: TIMEOUT_SECONDS,
        },
    }
}

fn entries<'call>(
    call: &'call VectorStoreEmbeddingCall,
    api_base: &'call str,
) -> (Vec<Entry<'call>>, Vec<Entry<'call>>) {
    let mut router_entries = Vec::new();
    for deployment in call.router.iter().flatten() {
        router_entries.push(entry(
            &deployment.model_name,
            &deployment.litellm_params.model,
            call,
            api_base,
        ));
    }
    let mut direct_entries = vec![entry(&call.model, &call.model, call, api_base)];
    direct_entries.extend(router_entries.iter().copied());
    (direct_entries, router_entries)
}

fn deployments(
    entries: Vec<Entry<'_>>,
    env: &BTreeMap<String, String>,
) -> Result<(Vec<ModelDeployment>, Vec<ChatDeployment>), SdkHttpError> {
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let text = serde_json::to_string(&Config {
        model_list: entries,
    })
    .map_err(SdkHttpError::Serialize)?;
    let config = loader
        .load(&text)
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let defaults = openai_defaults(&config);
    let resolved = config
        .model_list
        .iter()
        .map(|deployment| {
            ChatDeployment::resolve(deployment, &defaults)
                .map_err(|error| SdkHttpError::Deployment(Box::new(error)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((config.model_list, resolved))
}

fn chat_executor(client: &ProxiedClient, deployments: Vec<ChatDeployment>) -> ChatExecutor {
    ChatExecutor {
        client: client.clone(),
        deployments,
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(0)),
    }
}

// the injected router view: the case's model list answers both resolution and execution
struct FakeRouter {
    deployments: Vec<ModelDeployment>,
    direct: DirectEmbeddingExecutor,
}

#[async_trait]
impl VectorStoreRouter for FakeRouter {
    fn resolved_litellm_models(&self, model: &str, _team_id: Option<&str>) -> Vec<String> {
        self.deployments
            .iter()
            .filter(|deployment| deployment.model_name == model)
            .filter_map(|deployment| deployment.litellm_params.model.clone())
            .filter(|served| !served.is_empty())
            .collect()
    }

    fn model_list(&self) -> Vec<ModelDeployment> {
        self.deployments.clone()
    }

    async fn embedding(
        &self,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException> {
        let query = input.first().map_or("", String::as_str);
        self.direct
            .embed(model, query, &configuration)
            .await
            .map_err(|source| BaseLlmException {
                status_code: 502,
                message: source.to_string(),
                headers: Params::new(),
            })
    }
}
