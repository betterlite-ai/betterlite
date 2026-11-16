use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::mock::{chat::response as mock_chat, embedding::response as mock_embedding};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::stream::SseParser;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde::Serialize;
use uuid::Uuid;

use super::error::SdkHttpError;
use super::native::openai_defaults;
use super::result::{SdkResult, SyncRun};
use super::{ChatMessage, MessagesSyncStreamCall};

mod diagnostics;
mod run;
mod source;

#[derive(Serialize)]
struct WireChat<'wire> {
    model: &'wire str,
    messages: &'wire [ChatMessage],
    stream: bool,
}

#[derive(Serialize)]
struct Config<'config> {
    model_list: [Entry<'config>; 1],
}

#[derive(Serialize)]
struct Entry<'config> {
    model_name: &'config str,
    litellm_params: Params<'config>,
}

#[derive(Serialize)]
struct Params<'config> {
    model: String,
    api_base: &'config str,
    api_key: &'config str,
    timeout: u64,
}

pub(crate) async fn send(
    call: &MessagesSyncStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let executor = executor(call, api_base, env)?;
    let deployment = executor
        .deployments
        .first()
        .ok_or_else(|| SdkHttpError::Invalid(format!("no deployment serves {}", call.model)))?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for spec in &call.runs {
        let payloads = prefetch(&executor, deployment, call).await?;
        runs.push(run::run(call, spec, payloads)?);
    }
    Ok(SdkResult::MessagesSyncStream {
        value: SyncRun { runs },
    })
}

fn executor(
    call: &MessagesSyncStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<ChatExecutor, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    Ok(ChatExecutor {
        client: ProxiedClient::outbound(&policy)
            .map_err(|error| SdkHttpError::Client(Box::new(error)))?,
        deployments: deployments(call, api_base, env)?,
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(0)),
        clock: || 0,
        next_id: Uuid::new_v4,
        mock: MockSettings::default(),
        mock_chat,
        mock_embedding,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?,
    })
}

fn deployments(
    call: &MessagesSyncStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Vec<ChatDeployment>, SdkHttpError> {
    let config = Config {
        model_list: [Entry {
            model_name: &call.model,
            litellm_params: Params {
                model: format!("openai/{}", call.model),
                api_base,
                api_key: &call.api_key,
                timeout: call.timeout_seconds,
            },
        }],
    };
    let text = serde_json::to_string(&config).map_err(SdkHttpError::Serialize)?;
    let parsed = ConfigLoader {
        lookup: env.clone(),
    }
    .load(&text)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let defaults = openai_defaults(&parsed);
    let mut resolved = Vec::new();
    for entry in &parsed.model_list {
        let deployment = ChatDeployment::resolve(entry, &defaults)
            .map_err(|error| SdkHttpError::Deployment(Box::new(error)))?;
        resolved.push(deployment);
    }
    Ok(resolved)
}

// One POST per run, with the body the synchronous wrapper would send for the same case
async fn prefetch(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    call: &MessagesSyncStreamCall,
) -> Result<VecDeque<Vec<u8>>, SdkHttpError> {
    let wire = WireChat {
        model: &call.model,
        messages: &call.messages,
        stream: true,
    };
    let body = serde_json::to_vec(&wire).map_err(SdkHttpError::Serialize)?;
    let request =
        ChatRequest::parse(&body).map_err(|source| SdkHttpError::ChatParse(Box::new(source)))?;
    let response = executor
        .stream(deployment, &request)
        .await
        .map_err(|error| SdkHttpError::Chat(Box::new(error)))?;
    let mut stream = response.body_stream();
    let mut parser = SseParser::default();
    let mut payloads = VecDeque::new();
    while let Some(bytes) = stream.next().await {
        let bytes = bytes.map_err(|source| SdkHttpError::Stream(Box::new(source)))?;
        for event in parser
            .push(&bytes)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?
        {
            payloads.push_back(event);
        }
    }
    for event in parser.finish() {
        payloads.push_back(event);
    }
    Ok(payloads)
}
