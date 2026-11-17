use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::DropParams;
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
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_params::compose::{ConverseParamsRequest, map_openai_params};
use crate::converse_params::error::ConverseParamsError;
use crate::mantle_inputs::region::split_region_prefix;
use crate::model_routes::{ParamMap, ParamValue};

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

pub(super) fn params(source: &str) -> ParamMap {
    serde_json::from_str(source).expect("the fixture map parses")
}

pub(super) fn value(source: &str) -> ParamValue {
    serde_json::from_str(source).expect("the fixture value parses")
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture JSON parses")
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
    pub(super) budgets: Settings,
    pub(super) model_cost: ParamMap,
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
            budgets: Settings::default(),
            model_cost: params(model_cost),
        }
    }

    pub(super) const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime_catalog,
            packaged: &self.unlisted,
        }
    }

    pub(super) fn packaged(&self) -> &ModelCatalog {
        self.catalog.as_ref()
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

impl Session<'_> {
    fn environment(&mut self) -> CapabilityEnvironment<'_, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
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
        }
    }
}

pub(super) struct Call<'call> {
    pub(super) model: &'call str,
    pub(super) non_default: ParamMap,
    pub(super) optional: ParamMap,
    pub(super) drop: DropParams,
}

pub(super) struct Outcome {
    pub(super) params: Result<ParamMap, ConverseParamsError>,
    pub(super) logs: Vec<(Level, String)>,
}

impl Outcome {
    pub(super) fn mapped(self) -> ParamMap {
        self.params.expect("the parameters map")
    }

    pub(super) fn error(self) -> ConverseParamsError {
        self.params.expect_err("the mapping fails")
    }

    pub(super) fn debug(&self) -> Vec<&str> {
        self.logged(Level::DEBUG)
    }

    pub(super) fn warnings(&self) -> Vec<&str> {
        self.logged(Level::WARN)
    }

    pub(super) fn infos(&self) -> Vec<&str> {
        self.logged(Level::INFO)
    }

    fn logged(&self, level: Level) -> Vec<&str> {
        self.logs
            .iter()
            .filter(|(logged, _)| *logged == level)
            .map(|(_, message)| message.as_str())
            .collect()
    }
}

// The one path a case takes to production: same fixture, same sources, one call.
pub(super) fn run(
    fixture: &Fixture,
    session: &mut Session<'_>,
    answers: &Answers,
    call: Call<'_>,
) -> Outcome {
    let catalogs = fixture.catalogs();
    let mut environment = session.environment();
    let mut names = ToolNameMap::new();
    let mut optional = call.optional;
    let request = ConverseParamsRequest {
        model: call.model,
        drop: call.drop,
        non_default_params: &call.non_default,
        catalogs: &catalogs,
        budgets: &fixture.budgets,
        lookup: answers,
        packaged: fixture.packaged(),
    };
    let watch = Watch::default();
    let guard = watch.install();
    let params = runtime().block_on(map_openai_params(
        &request,
        &mut optional,
        &mut environment,
        &mut names,
    ));
    drop(guard);
    Outcome {
        params: params.map(|()| optional),
        logs: watch.messages(),
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

impl<S: Subscriber> Layer<S> for Watch {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, S>) {
        let mut line = Line::default();
        event.record(&mut line);
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                *event.metadata().level(),
                line.0.trim_matches('"').to_owned(),
            ));
    }
}
