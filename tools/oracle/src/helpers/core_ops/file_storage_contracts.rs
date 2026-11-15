use std::convert::Infallible;
use std::sync::Arc;

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_storage_contracts::batch::responses_batch_to_chat_body;
use betterlite_core::api::file_storage_contracts::storage::storage_url_to_row_id;
use betterlite_core::api::responses_chat_options::RequestAssembly;
use betterlite_core::api::responses_chat_options::capabilities::CompletionBridgeProbe;
use betterlite_core::responses_bridge::{BridgeRequest, ModelInfoError};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;
use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    StorageRowId(StorageRowIdCall),
    ResponsesBatch(ResponsesBatchCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageRowIdCall {
    storage_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponsesBatchCall {
    body: Json,
    custom_llm_provider: Option<String>,
}

struct NoParameters;

impl SupportedParameterSource for NoParameters {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

struct NoProbe;

impl CompletionBridgeProbe for NoProbe {
    fn bridges_back(&self, _request: &BridgeRequest<'_>) -> Result<bool, ModelInfoError> {
        Ok(false)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cache = tool_cache()?;
        let answers = input
            .calls
            .iter()
            .map(|call| answer(call, &cache))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(Value::Array(answers))
    }
}

fn tool_cache() -> Result<DualLocalCache, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    Ok(DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        Diagnostics {
            redactor: Arc::new(redactor),
            emit: Arc::new(|_event| {}),
        },
    ))
}

fn answer(call: &Call, cache: &DualLocalCache) -> Result<Value, Box<HelperError>> {
    match call {
        Call::StorageRowId(payload) => Ok(row_id_answer(&payload.storage_url)),
        Call::ResponsesBatch(payload) => batch_answer(payload, cache),
    }
}

fn row_id_answer(storage_url: &str) -> Value {
    match storage_url_to_row_id(storage_url) {
        Ok(row_id) => json!({"row_id": row_id}),
        Err(error) => json!({"message": error.to_string()}),
    }
}

fn batch_answer(
    call: &ResponsesBatchCall,
    cache: &DualLocalCache,
) -> Result<Value, Box<HelperError>> {
    let parameters = NoParameters;
    let probe = NoProbe;
    let mut assembly = RequestAssembly {
        limits: ApiLimits::default(),
        cache,
        parameters: &parameters,
        probe: &probe,
        logging: None,
        route_all_openai_to_responses: false,
    };
    match responses_batch_to_chat_body(
        &call.body,
        call.custom_llm_provider.as_deref(),
        &mut assembly,
    ) {
        Ok(request) => Ok(serde_json::to_value(request)?),
        Err(error) => Ok(json!({"message": error.to_string()})),
    }
}
