use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::bedrock_model_capabilities::ModelInfoLookup;
use betterlite_bedrock::model_routes::{ParamMap, ParamValue};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use indexmap::IndexMap;
use secrecy::SecretString;

pub(super) fn helper_error<E>(error: E, status: Option<u16>) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockServicesBatchRecords".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// The lookup resolves nothing, so every capability answer comes from the model id alone.
pub(super) struct Sources {
    catalog: ModelCatalog,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Sources {
    pub(super) fn new() -> Result<Self, Box<HelperError>> {
        Ok(Self {
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .map_err(|error| helper_error(error, None))?,
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
        })
    }

    pub(super) fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
        }
    }
}

// LiteLLM reads litellm.model_cost[model], so the stub answers from the case map it was handed.
pub(super) struct CaseLookup<'case> {
    pub(super) model_cost: &'case ParamMap,
}

fn decision_json(value: &ParamValue) -> Json {
    match value {
        ParamValue::Null => Json::Null,
        ParamValue::Bool(flag) => Json::Bool(*flag),
        ParamValue::Number(number) => Json::Number(number.clone()),
        ParamValue::Text(text) => Json::Text(text.clone()),
        ParamValue::List(items) => Json::List(items.iter().map(decision_json).collect()),
        ParamValue::Mapping(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), decision_json(entry)))
                .collect(),
        ),
    }
}

impl ModelInfoLookup for CaseLookup<'_> {
    type Error = Infallible;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Infallible>> + Send {
        let fields: IndexMap<String, Json> = match self.model_cost.get(model) {
            Some(ParamValue::Mapping(entry)) => entry
                .iter()
                .map(|(name, value)| (name.clone(), decision_json(value)))
                .collect(),
            Some(
                ParamValue::Null
                | ParamValue::Bool(_)
                | ParamValue::Number(_)
                | ParamValue::Text(_)
                | ParamValue::List(_),
            )
            | None => IndexMap::new(),
        };
        ready(Ok(ModelInfo { fields }))
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or("now is out of range")
        .or_value_error()
}
