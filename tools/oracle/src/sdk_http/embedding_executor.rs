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
use betterlite_models::decisions::Json;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::api::vector_store_contracts::config::{
    EmbeddingEntry, Params, VectorStoreEmbeddingExecutor,
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
    let (router_config, router_deployments, defaults) =
        deployments(router_entries(call, api_base), env)?;
    // the direct side serves no deployment: its key and base come from the configuration and the env
    let direct = DirectEmbeddingExecutor {
        executor: chat_executor(&client, Vec::new()),
        deployments: Vec::new(),
        defaults: defaults.clone(),
    };
    let outcomes = if call.router.is_some() {
        let handle = FakeRouter {
            direct: DirectEmbeddingExecutor {
                executor: chat_executor(&client, router_deployments.clone()),
                deployments: router_deployments,
                defaults,
            },
            deployments: router_config,
        };
        let executor = RouterEmbeddingExecutor {
            router: handle,
            metadata: call.metadata.clone(),
            direct,
        };
        both_modes(&executor, call, api_base).await
    } else {
        both_modes(&direct, call, api_base).await
    };
    let projected = outcomes.map_err(|source| SdkHttpError::Embedding {
        source: Box::new(source),
    })?;
    Ok(SdkResult::Embedding { value: projected })
}

async fn both_modes<E>(
    executor: &E,
    call: &VectorStoreEmbeddingCall,
    api_base: &str,
) -> Result<EmbeddingOutcomes, VectorStoreEmbeddingError>
where
    E: VectorStoreEmbeddingExecutor,
{
    // the harness owns api_base, and a direct call has no deployment to take it from
    let mut configuration = call.configuration.clone();
    configuration.insert("api_base".to_owned(), Json::Text(api_base.to_owned()));
    let sync = executor
        .embed(
            EmbeddingEntry::Embedding,
            &call.model,
            &call.query,
            &configuration,
        )
        .await?;
    let asynchronous = executor
        .embed(
            EmbeddingEntry::Aembedding,
            &call.model,
            &call.query,
            &configuration,
        )
        .await?;
    Ok(EmbeddingOutcomes { sync, asynchronous })
}

fn router_entries<'call>(
    call: &'call VectorStoreEmbeddingCall,
    api_base: &'call str,
) -> Vec<Entry<'call>> {
    call.router
        .iter()
        .flatten()
        .map(|deployment| Entry {
            model_name: &deployment.model_name,
            litellm_params: EntryParams {
                model: &deployment.litellm_params.model,
                api_base,
                api_key: &deployment.litellm_params.api_key,
                timeout: TIMEOUT_SECONDS,
            },
        })
        .collect()
}

fn deployments(
    entries: Vec<Entry<'_>>,
    env: &BTreeMap<String, String>,
) -> Result<(Vec<ModelDeployment>, Vec<ChatDeployment>, OpenAiDefaults), SdkHttpError> {
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
    Ok((config.model_list, resolved, defaults))
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

    // the deployment the router picked is the one in its model list, with its own key and base
    async fn embedding(
        &self,
        entry: EmbeddingEntry,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException> {
        let query = input.first().map_or("", String::as_str);
        self.direct
            .embed(entry, model, query, &configuration)
            .await
            .map_err(|source| BaseLlmException {
                status_code: 502,
                message: source.to_string(),
                headers: Params::new(),
            })
    }
}
