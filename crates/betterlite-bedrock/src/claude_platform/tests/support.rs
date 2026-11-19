#![cfg_attr(
    test,
    expect(
        clippy::disallowed_types,
        reason = "the decoded chunks compare as serde_json values"
    )
)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::request::error::RequestError;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use tokio::runtime::Builder;

use betterlite_anthropic::stream_decode::error::ReceiveError;
use betterlite_anthropic::stream_decode::{DecodedChunk, TransportChunk};
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use uuid::Uuid;

use crate::claude_platform::chat::{
    ChatHeadersRequest, ChatPrepared, ChatRequest, ChatSources, HeaderSources, transform_request,
    validate_environment,
};
use crate::claude_platform::error::{ChatValidationError, NativeTransformError};
use crate::claude_platform::native::transform_request as native_transform;
use crate::claude_platform::native::{NativePrepared, NativeRequest, NativeSources};
use crate::claude_platform::stream::{StreamFlags, get_model_response_iterator};
use crate::mantle_inputs::region::split_region_prefix;

const ID: &str = "generated-id";
pub(crate) const MAX_EVENT_BYTES: usize = 0x0010_0000;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

// The cases name no huggingface model, so the transport refuses instead of reaching the network.
pub(crate) struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(crate) struct NoHfError;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}

pub(crate) struct RefusingFetcher;

fn refused(url: &str) -> ImageFetchError {
    ImageFetchError::Disabled {
        url: url.to_owned(),
    }
}

impl RemoteMediaFetcher for RefusingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(refused(url)))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(refused(urls.first().map_or("", String::as_str))))
    }
}

pub(crate) struct NoFileBlocks;

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file blocks")]
pub(crate) struct NoFileError;

impl FileBlocks for NoFileBlocks {
    type Failure = NoFileError;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileError> {
        Err(NoFileError)
    }
}

pub(crate) struct Records {
    map: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    packaged: RawCatalog,
    budgets: Settings,
}

impl Records {
    pub(crate) fn empty() -> Self {
        let raw = serde_json::json!({});
        Self {
            map: ModelCostMap::parse(b"{}").expect("the empty model cost map parses"),
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("the empty typed catalog parses"),
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns build"),
            runtime: serde_json::from_value(raw.clone()).expect("the empty raw catalog parses"),
            packaged: serde_json::from_value(raw).expect("the empty raw catalog parses"),
            budgets: Settings::default(),
        }
    }

    pub(crate) const fn catalog(&self) -> &ModelCatalog {
        &self.typed
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.packaged,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    const fn environment<'fixture>(
        &'fixture self,
        resolution: &'fixture ResolutionContext<'fixture>,
        keys: &'fixture mut ModelCostKeys,
        dynamic: &'fixture NoDynamicMetadata,
    ) -> CapabilityEnvironment<'fixture, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.map.entries,
                vertex: &self.map.vertex,
                generalizations: &self.generalizations,
                resolution,
                normalizers: NORMALIZERS,
            },
            keys,
            dynamic,
            hf: &self.hf,
            provider_catalog: &self.provider_catalog,
            provider_generation: 0,
        }
    }

    fn parts(
        &self,
    ) -> (
        CatalogSources<'_>,
        ModelCostKeys,
        NoDynamicMetadata,
        ResolutionContext<'_>,
    ) {
        (
            self.catalogs(),
            ModelCostKeys::default(),
            NoDynamicMetadata,
            self.resolution(),
        )
    }

    pub(crate) fn chat_headers(
        &self,
        request: &ChatHeadersRequest<'_>,
    ) -> Result<AnthropicHeaders, ChatValidationError> {
        let (catalogs, mut keys, dynamic, resolution) = self.parts();
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        let mut sources = HeaderSources {
            environment: &mut environment,
            catalogs: &catalogs,
        };
        block_on(validate_environment(request, &mut sources))
    }

    pub(crate) fn chat_body(
        &self,
        request: &ChatRequest<'_>,
    ) -> Result<ChatPrepared, RequestError<NoFileError>> {
        let (catalogs, mut keys, dynamic, resolution) = self.parts();
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        let fetcher = RefusingFetcher;
        let blocks = NoFileBlocks;
        let new_id = || ID.to_owned();
        let prompts = PromptDeps {
            fetcher: &fetcher,
            blocks: &blocks,
            new_id: &new_id,
        };
        block_on(transform_request(
            request,
            &mut ChatSources {
                environment: &mut environment,
                catalogs: &catalogs,
                prompts: &prompts,
            },
        ))
    }

    pub(crate) fn native_body(
        &self,
        request: &NativeRequest<'_>,
    ) -> Result<NativePrepared, NativeTransformError> {
        let (catalogs, mut keys, dynamic, resolution) = self.parts();
        let mut environment = self.environment(&resolution, &mut keys, &dynamic);
        block_on(native_transform(
            request,
            &mut NativeSources {
                environment: &mut environment,
                catalogs: &catalogs,
                budgets: &self.budgets,
            },
        ))
    }
}

pub(crate) fn block_on<Output, Fut>(future: Fut) -> Output
where
    Fut: Future<Output = Output>,
{
    let runtime = Builder::new_current_thread().build();
    runtime.expect("the test runtime builds").block_on(future)
}

pub(crate) fn defaults() -> OptionDefaults {
    OptionDefaults::default()
}

pub(crate) struct FixedIds {
    value: Uuid,
}

impl FixedIds {
    pub(crate) fn new(value: &str) -> Self {
        Self {
            value: Uuid::parse_str(value).expect("the case id is a uuid"),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.value
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the case counts no reasoning tokens")]
pub(crate) struct CounterError;

pub(crate) struct Counting;

impl ReasoningTokenCounter for Counting {
    type Failure = CounterError;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, CounterError> {
        Ok(0)
    }
}

// The decoded chunks a case walks, as plain JSON the assertions and the adapter both read.
pub(crate) fn stream_chunks(
    steps: Vec<TransportChunk>,
    json_mode: Option<bool>,
    ids: &FixedIds,
) -> Vec<serde_json::Value> {
    let stream = get_model_response_iterator(
        steps.into_iter().map(Ok::<_, ReceiveError<CounterError>>),
        StreamFlags {
            sync_stream: false,
            json_mode,
        },
        Counting,
        ids,
        MAX_EVENT_BYTES,
    );
    let mut recorded = Vec::new();
    for outcome in stream {
        match outcome {
            Ok(DecodedChunk::Event(event)) => recorded.push(event_value(&event)),
            Ok(DecodedChunk::Generic(generic)) => recorded.push(generic_value(&generic)),
            Err(error) => recorded.push(serde_json::json!({"error": error.to_string()})),
        }
    }
    recorded
}

pub(crate) fn event_value(event: &StreamEventChunk) -> serde_json::Value {
    let choice = event.choices.first();
    let delta = choice.map(|selected| &selected.delta);
    serde_json::json!({
        "id": &event.id,
        "model": &event.model,
        "content": delta.map(|part| part.content.clone()).unwrap_or_default(),
        "finish_reason": choice.and_then(|selected| selected.finish_reason.clone()),
    })
}

pub(crate) fn generic_value(chunk: &GenericChunk) -> serde_json::Value {
    serde_json::json!({"text": &chunk.text, "is_finished": chunk.is_finished})
}
