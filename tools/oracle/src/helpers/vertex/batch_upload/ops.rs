use std::convert::Infallible;
use std::future::{Future, ready};
use std::pin::pin;
use std::sync::Arc;

use betterlite_config::sections::local_ttl_cache::Settings as CacheSettings;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_context::error::ContextError;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_context::provider_message_primitives::{Record, developer_to_system};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_storage_contracts::batch::{
    BatchBodyError, responses_batch_to_chat_body,
};
use betterlite_core::api::responses_chat_options::RequestAssembly;
use betterlite_core::api::responses_chat_options::capabilities::CompletionBridgeProbe;
use betterlite_core::responses_bridge::{BridgeRequest, ModelInfoError};
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_vertex::gemini_options::defaults::OptionSettings;
use betterlite_vertex::gemini_thinking::settings::{
    MinimalEffortSupport, ThinkingBudgets, ThinkingSettings,
};
use betterlite_vertex::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup,
};
use betterlite_vertex::vertex_batch_upload::rows::{RowContext, RowMappers};
use betterlite_vertex::vertex_batch_upload::sources::{FileContent, FileTypes};
use betterlite_vertex::vertex_batch_upload::upload::vertex_batch_upload;
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const MAX_LINE_BYTES: usize = 1_048_576;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    content: String,
    passthrough: Json,
}

#[derive(Debug, Error)]
enum MapperFailure {
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    Batch(#[from] BatchBodyError),
    #[error(transparent)]
    Context(#[from] ContextError),
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

// B1025's Responses bridge and B010's developer projection, which the binder above the Vertex crate injects
struct Mappers {
    cache: DualLocalCache,
}

impl RowMappers for Mappers {
    type Error = MapperFailure;

    fn responses_to_chat(&self, provider: &str, body: &Object) -> Result<Object, MapperFailure> {
        let body: DecisionsJson = serde_json::from_slice(&serde_json::to_vec(body)?)?;
        let mut assembly = RequestAssembly {
            limits: ApiLimits::default(),
            cache: &self.cache,
            parameters: &NoParameters,
            probe: &NoProbe,
            logging: None,
            route_all_openai_to_responses: false,
        };
        let request = responses_batch_to_chat_body(&body, Some(provider), &mut assembly)?;
        Ok(serde_json::from_slice(&serde_json::to_vec(&request)?)?)
    }

    fn developer_to_system(&self, messages: Vec<Object>) -> Result<Vec<Object>, MapperFailure> {
        let records: Vec<Record<Value>> = serde_json::from_slice(&serde_json::to_vec(&messages)?)?;
        let projected = developer_to_system(records)?;
        Ok(serde_json::from_slice(&serde_json::to_vec(&projected)?)?)
    }
}

// the case fixes what the model catalog answers, so the comparison fixes the capability input
struct Catalog;

impl ModelCapabilityLookup for Catalog {
    fn supports(
        &self,
        _kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        Ok(true)
    }
}

impl MinimalEffortSupport for Catalog {
    fn disables_minimal(&self, _model: &str) -> bool {
        false
    }
}

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let mappers = Mappers {
        cache: DualLocalCache::new(
            None,
            None,
            &CacheSettings::default(),
            Arc::new(|| 0.0_f64),
            Diagnostics {
                redactor: Arc::new(redactor),
                emit: Arc::new(|_event| {}),
            },
        ),
    };
    let options = OptionSettings::default();
    let schema = SchemaSettings::default();
    let thinking = ThinkingSettings {
        budgets: ThinkingBudgets {
            minimal: 128,
            minimal_flash: 1,
            minimal_pro: 128,
            minimal_flash_lite: 512,
            low: 1024,
            medium: 2048,
            high: 4096,
            disable: 0,
        },
        default_thinking_level_low: false,
    };
    let context = RowContext {
        mappers: &mappers,
        fetcher: &NoFetch,
        capabilities: &Catalog,
        options: &options,
        schema: &schema,
        thinking: &thinking,
        support: &Catalog,
        modify_params: false,
    };
    let mut answers = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        answers.push(runtime.block_on(answer(call, &context))?);
    }
    Ok(Value::Array(answers))
}

async fn answer(
    call: &Call,
    context: &RowContext<'_, Mappers, NoFetch>,
) -> Result<Value, Box<HelperError>> {
    let file = FileTypes::Content(FileContent::Bytes(call.content.clone().into_bytes()));
    let mut stream = pin!(vertex_batch_upload(
        file,
        "batch",
        &call.passthrough,
        context,
        MAX_LINE_BYTES
    ));
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks.push(String::from_utf8(chunk.or_value_error()?).or_value_error()?);
    }
    Ok(json!({ "chunks": chunks }))
}
