mod fixture;
mod output;
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::embeddings::call::{ClientContext, Dependencies, EmbeddingCall, embedding};
use betterlite_azure::embeddings::reply::ModelResponse;
use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use fixture::{Answer, Cache, ClientName, NoFactory, NoSources, Recorder, Sdk};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    input: EmbeddingInput,
    answer: Answer,
    client: ClientName,
    #[serde(default)]
    aembedding: bool,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default = "default_api_base")]
    api_base: String,
    #[serde(default = "default_api_version")]
    api_version: String,
    #[serde(default = "default_timeout")]
    timeout: f64,
    #[serde(default = "default_api_key")]
    api_key: String,
    #[serde(default)]
    azure_ad_token: Option<String>,
    #[serde(default)]
    max_retries: Option<i64>,
    #[serde(default)]
    optional_params: IndexMap<String, Json>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    litellm_params: LiteLlmParams,
}

fn default_model() -> String {
    "my-deployment".to_owned()
}

fn default_api_base() -> String {
    "https://example.openai.azure.com".to_owned()
}

fn default_api_version() -> String {
    "2024-02-01".to_owned()
}

const fn default_timeout() -> f64 {
    30.0
}

fn default_api_key() -> String {
    "azure-key".to_owned()
}

async fn run_step(step: &Step) -> Value {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let client = Sdk {
        kind: step.client.kind(),
        answer: step.answer.clone(),
        calls: Arc::clone(&calls),
    };
    let settings = Settings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = NoSources;
    let mut cache = Cache::default();
    let mut refresh_cache = RefreshProviderCache::new(&sources);
    let mut recorder = Recorder::default();
    let call = EmbeddingCall {
        model: &step.model,
        input: &step.input,
        api_base: &step.api_base,
        api_version: Some(&step.api_version),
        timeout: step.timeout,
        optional_params: step.optional_params.clone(),
        api_key: Some(SecretString::from(step.api_key.clone())),
        azure_ad_token: step.azure_ad_token.clone().map(SecretString::from),
        max_retries: step.max_retries,
        client: Some(&client),
        aembedding: step.aembedding,
        headers: step.headers.clone(),
        litellm_params: step.litellm_params.clone(),
        model_response: ModelResponse::Embedding,
    };
    let mut dependencies = Dependencies {
        factory: &NoFactory,
        context: ClientContext {
            provider: None,
            settings: &settings,
            environment: &environment,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
            sources: &sources,
        },
        cache: &mut cache,
        refresh_cache: &mut refresh_cache,
        logger: &mut recorder,
    };
    let outcome = embedding(call, &mut dependencies).await;
    let seen = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    json!({
        "result": outcome.as_ref().ok().and_then(Option::as_ref).map(output::reply),
        "raised": outcome.as_ref().err().map(output::raised),
        "pre_call": recorder.pre,
        "post_call": recorder.post,
        "calls": seen,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let steps: Vec<Value> = input
            .steps
            .iter()
            .map(|step| runtime.block_on(run_step(step)))
            .collect();
        Ok(json!({"steps": steps}))
    }
}
