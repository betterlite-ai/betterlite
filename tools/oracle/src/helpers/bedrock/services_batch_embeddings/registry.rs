use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_bedrock::bedrock_model_capabilities::ModelInfoLookup;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::full_model_metadata::error::FullMetadataError;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, ModelCostMapError, NameNormalizers,
};
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;

const COST_MAP_NAMES: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, thiserror::Error)]
pub(super) enum RegistryError {
    #[error("the fixture model cost map is invalid: {0}")]
    CostMap(#[from] ModelCostMapError),
    #[error("the empty typed catalog is invalid: {0}")]
    Catalog(#[from] CatalogError),
    #[error("the fixture model cost map does not serialize: {0}")]
    Json(#[from] serde_json::Error),
}

// The cases name no huggingface id, so the transport refuses instead of reaching the network.
struct NoFetch;

#[derive(Debug, thiserror::Error)]
#[error("the fixture must not fetch a Hugging Face config")]
struct NoFetchError;

impl ModelInfoClient for NoFetch {
    type Error = NoFetchError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoFetchError>> + Send {
        ready(Err(NoFetchError))
    }
}

// The shared lower metadata owner over a fixture cost map, with no catalog, provider overlay or transport behind it.
pub(super) struct Registry {
    entries: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    names: BTreeSet<String>,
    providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    hf: HfConfigClient<NoFetch>,
}

impl Registry {
    pub(super) fn load(model_cost: &ParamMap) -> Result<Self, RegistryError> {
        let bytes = serde_json::to_vec(model_cost)?;
        Ok(Self {
            entries: ModelCostMap::parse(&bytes)?,
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            generalizations: CatalogGeneralizations::default(),
            names: BTreeSet::new(),
            providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            hf: HfConfigClient {
                client: NoFetch,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
        })
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.names,
            custom_providers: &self.providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn metadata_environment<'registry>(
        &'registry self,
        resolution: &'registry ResolutionContext<'registry>,
    ) -> MetadataEnvironment<'registry> {
        MetadataEnvironment {
            model_cost: &self.entries.entries,
            vertex: &self.entries.vertex,
            generalizations: &self.generalizations,
            resolution,
            normalizers: COST_MAP_NAMES,
        }
    }
}

impl ModelInfoLookup for Registry {
    type Error = FullMetadataError<Infallible>;

    async fn bedrock_model_info(&self, model: &str) -> Result<ModelInfo, Self::Error> {
        let resolution = self.resolution();
        let environment = self.metadata_environment(&resolution);
        let dynamic = NoDynamicMetadata;
        let request = MetadataRequest {
            model,
            custom_llm_provider: Some("bedrock"),
            api_base: None,
            api_key: None,
        };
        let mut keys = ModelCostKeys::default();
        let metadata =
            get_model_info_helper(&request, &environment, &mut keys, &dynamic, &self.hf).await?;
        Ok(ModelInfo::parse(&metadata)?)
    }
}
