use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::full_model_metadata::record::ModelInfo;
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
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::mantle_inputs::region::split_region_prefix;
use crate::message_options::effort::clamp_adaptive_effort;
use crate::message_options::thinking::{ensure_clear_thinking, supports_extended_thinking};
use crate::message_options::{MessageOptionsError, OptionsSources};
use crate::model_routes::ParamMap;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

const LOG_PREFIX: &str = "Bedrock clear_thinking_20251015:";

pub(super) fn map(source: &str) -> ParamMap {
    serde_json::from_str(source).expect("fixture map parses")
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

pub(super) struct Fixture {
    entries: ModelCostMap,
    catalog: Arc<ModelCatalog>,
    typed: ModelCatalog,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    variables: BTreeMap<String, String>,
    hf: HfConfigClient<NoHfTransport>,
    provider_catalog: Catalog,
    patterns: CandidatePatterns,
    runtime_catalog: RawCatalog,
    unlisted: RawCatalog,
}

impl Fixture {
    pub(super) fn new(model_cost: &str) -> Self {
        let mut bundled = BundledCatalog::default();
        Self {
            entries: ModelCostMap::parse(model_cost.as_bytes()).expect("the model cost map parses"),
            catalog: bundled.get(),
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            variables: BTreeMap::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            provider_catalog: Catalog::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime_catalog: serde_json::from_slice(model_cost.as_bytes())
                .expect("the raw catalog parses"),
            unlisted: RawCatalog::new(),
        }
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime_catalog,
            packaged: &self.unlisted,
        }
    }

    pub(super) fn session(&self) -> Session<'_> {
        let resolution = ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.variables,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        Session {
            fixture: self,
            resolution,
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
        }
    }
}

pub(super) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
}

// One unit of test work: the production call and the request or params it reads.
pub(super) enum Job<'request> {
    Ensure(&'request str, &'request mut ParamMap),
    Clamp(&'request str, &'request mut ParamMap, &'request Answers),
    Supports(&'request [&'request str]),
}

// What the job answered, in the shape its production call returns.
pub(super) enum Outcome {
    Injected(Result<bool, MessageOptionsError>),
    Clamped(Result<(), MessageOptionsError>),
    Supported(Vec<bool>),
}

impl Outcome {
    pub(super) fn injected(self) -> Option<Result<bool, MessageOptionsError>> {
        match self {
            Self::Injected(result) => Some(result),
            Self::Clamped(_) | Self::Supported(_) => None,
        }
    }

    pub(super) fn clamped(self) -> Option<Result<(), MessageOptionsError>> {
        match self {
            Self::Clamped(result) => Some(result),
            Self::Injected(_) | Self::Supported(_) => None,
        }
    }

    pub(super) fn supported(self) -> Option<Vec<bool>> {
        match self {
            Self::Supported(result) => Some(result),
            Self::Injected(_) | Self::Clamped(_) => None,
        }
    }
}

impl Session<'_> {
    // The one place a request's sources are built, so every question runs over the same fixture.
    pub(super) fn ask(&mut self, budgets: &Settings, job: Job<'_>) -> Outcome {
        let catalogs = self.fixture.catalogs();
        let mut environment = CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.fixture.entries.entries,
                vertex: &self.fixture.entries.vertex,
                generalizations: &self.fixture.catalog.generalizations,
                resolution: &self.resolution,
                normalizers: NORMALIZERS,
            },
            keys: &mut self.keys,
            dynamic: &self.dynamic,
            hf: &self.fixture.hf,
            provider_catalog: &self.fixture.provider_catalog,
            provider_generation: 0,
        };
        let mut sources = OptionsSources {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets,
        };
        let runtime = runtime();
        match job {
            Job::Ensure(model, request) => Outcome::Injected(
                runtime.block_on(ensure_clear_thinking(request, model, &mut sources)),
            ),
            Job::Clamp(model, params, answers) => Outcome::Clamped(runtime.block_on(
                clamp_adaptive_effort(model, params, &mut sources, answers, &self.fixture.catalog),
            )),
            Job::Supports(models) => {
                let mut supported = Vec::new();
                for model in models {
                    supported
                        .push(runtime.block_on(supports_extended_thinking(model, &mut sources)));
                }
                Outcome::Supported(supported)
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

pub(super) struct Answers(BTreeMap<String, IndexMap<String, Json>>);

impl Answers {
    pub(super) fn new(entries: &[(&str, &str)]) -> Self {
        Self(
            entries
                .iter()
                .map(|(model, fields)| {
                    (
                        (*model).to_owned(),
                        serde_json::from_str(fields).expect("model info parses"),
                    )
                })
                .collect(),
        )
    }
}

impl ModelInfoLookup for Answers {
    type Error = Unmapped;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Unmapped>> + Send {
        let answer = self.0.get(model).cloned();
        ready(answer.map(|fields| ModelInfo { fields }).ok_or(Unmapped))
    }
}

pub(super) struct NoHfTransport;

impl ModelInfoClient for NoHfTransport {
    type Error = Unmapped;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Unmapped>> + Send {
        ready(Err(Unmapped))
    }
}

#[derive(Clone, Default)]
pub(super) struct Watch {
    events: Arc<Mutex<Vec<(Level, String)>>>,
}

impl Watch {
    pub(super) fn install(&self) -> DefaultGuard {
        let subscriber = tracing_subscriber::registry().with(self.clone());
        set_default(subscriber)
    }

    pub(super) fn messages(&self) -> Vec<(Level, String)> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Watch {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        let mut line = Line::default();
        event.record(&mut line);
        if line.0.starts_with(LOG_PREFIX) {
            let level = *event.metadata().level();
            self.events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((level, line.0));
        }
    }
}
