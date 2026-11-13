use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
// ast-grep-ignore: no-long-names
use betterlite_router::semantic_embedding_inputs::build_router_embedding_metadata as router_metadata;
// ast-grep-ignore: no-long-names
use betterlite_router::semantic_embedding_inputs::resolve_embedding_max_input_tokens as max_tokens_for;
use betterlite_router::semantic_embedding_inputs::{
    ConfiguredTokenLimits, MetadataMap, ModelListEntry, resolve_embedding_router,
    resolve_embedding_timeout, truncate_embedding_input,
};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Router {
        embedding_model: String,
        model_list: Option<Vec<ModelRow>>,
        router_present: bool,
    },
    Metadata {
        metadata: Option<MetadataMap>,
    },
    MaxInputTokens {
        configured: Option<i64>,
        embedding_model: String,
        model_list: Option<Vec<ModelRow>>,
        router_present: bool,
    },
    Timeout {
        configured: Option<f64>,
        default_seconds: f64,
    },
    Truncate {
        prompt: String,
        embedding_model: String,
        max_input_tokens: Option<i64>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelRow {
    model_name: Option<String>,
    model_info: Option<ModelInfoRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelInfoRow {
    max_input_tokens: Option<i64>,
    max_output_tokens: Option<i64>,
}

struct RouterMarker;

struct ModelListLimits<'rows> {
    rows: &'rows [ModelRow],
}

impl ConfiguredTokenLimits for ModelListLimits<'_> {
    fn get_configured_token_limits(&self, model_name: &str) -> (Option<i64>, Option<i64>) {
        let found = self
            .rows
            .iter()
            .find(|row| row.model_name.as_deref() == Some(model_name));
        let Some(model_info) = found.and_then(|row| row.model_info.as_ref()) else {
            return (None, None);
        };
        (model_info.max_input_tokens, model_info.max_output_tokens)
    }
}

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Router {
                embedding_model,
                model_list,
                router_present,
            } => Ok(router_resolution(
                embedding_model,
                model_list.as_deref(),
                *router_present,
            )),
            Input::Metadata { metadata } => Ok(json!({
                "metadata": router_metadata(metadata.as_ref()),
            })),
            Input::MaxInputTokens {
                configured,
                embedding_model,
                model_list,
                router_present,
            } => Ok(max_input_tokens(
                *configured,
                embedding_model,
                model_list.as_deref(),
                *router_present,
            )),
            Input::Timeout {
                configured,
                default_seconds,
            } => Ok(json!({
                "timeout": resolve_embedding_timeout(*configured, *default_seconds),
            })),
            Input::Truncate {
                prompt,
                embedding_model,
                max_input_tokens,
            } => truncate(prompt, embedding_model, *max_input_tokens),
        }
    }
}

fn router_resolution(
    embedding_model: &str,
    model_list: Option<&[ModelRow]>,
    router_present: bool,
) -> serde_json::Value {
    let marker = RouterMarker;
    let entries: Option<Vec<ModelListEntry>> =
        model_list.map(|rows| rows.iter().map(entry).collect());
    let resolved = resolve_embedding_router(
        embedding_model,
        router_present.then_some(&marker),
        entries.as_deref(),
    );
    json!({"resolved": resolved.is_some()})
}

fn max_input_tokens(
    configured: Option<i64>,
    embedding_model: &str,
    model_list: Option<&[ModelRow]>,
    router_present: bool,
) -> serde_json::Value {
    let deployment = if router_present {
        model_list.map(|rows| ModelListLimits { rows })
    } else {
        None
    };
    let resolved = deployment.as_ref().map_or_else(
        || max_tokens_for(configured, embedding_model, None),
        |found| max_tokens_for(configured, embedding_model, Some(found)),
    );
    json!({"max_input_tokens": resolved})
}

fn truncate(prompt: &str, embedding_model: &str, max_input_tokens: Option<i64>) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(error.to_string()))?;
    let selector = selector()?;
    let truncated = runtime
        .block_on(truncate_embedding_input(
            &selector,
            prompt,
            embedding_model,
            max_input_tokens,
        ))
        .map_err(|error| failure(error.to_string()))?;
    Ok(json!({"prompt": truncated}))
}

fn entry(row: &ModelRow) -> ModelListEntry {
    ModelListEntry {
        model_name: row.model_name.clone(),
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn hub_settings() -> HubSettings {
    HubSettings {
        endpoint: "https://huggingface.co".to_owned(),
        offline: OfflineSetting::Offline,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: PathBuf::new(),
        hub_cache: PathBuf::new(),
        token_path: PathBuf::new(),
        stored_tokens_path: PathBuf::new(),
        transfer: TransferSetting::Standard,
    }
}

// offline with a bundled catalog, so truncation reads no network and no token files
fn selector() -> Result<Selector, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|error| failure(error.to_string()))?;
    let client = Arc::new(
        ProxiedClient::outbound(&Arc::new(policy)).map_err(|error| failure(error.to_string()))?,
    );
    let settings = hub_settings();
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let tokens = Arc::new(HubTokenResolver::new(
        settings.clone(),
        OidcSettings::default(),
        Arc::clone(&client),
        reader,
        clock,
    ));
    let hub = Arc::new(HubDownloader::new(
        Arc::new(settings),
        tokens,
        Arc::new(Instant::now),
    ));
    let facility: Arc<dyn BlockingCounter> = Arc::new(InlineCounter);
    let counter = Arc::new(CountingService::new(4, facility, EncodeLimits::default()));
    let fetcher = Arc::new(TiktokenFetcher::new(client, None, Arc::clone(&counter)));
    let catalog = BundledCatalog::default().get();
    Ok(Selector::new(fetcher, hub, catalog, counter, false))
}
