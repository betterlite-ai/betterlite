use std::collections::BTreeSet;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelInfoLookup, ModelRegistry,
    RegistrationEnvironment,
};
use serde::Deserialize;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::deployment_identity::construct::{
    Construction, ConstructionPolicy, DeploymentInput, create_deployment, ignore_or_raise,
};
use crate::deployment_identity::error::{DeploymentError, InvalidDeployment};
use crate::deployment_identity::passthrough::{
    PassThroughCall, StripProvider, endpoint_for_deployment, vertex_pass_through,
};
use crate::deployment_identity::stable_id::generate_model_id;
use crate::deployment_pricing::PricingInputs;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn is_auto_router(model: &str) -> bool {
    model.starts_with("auto_router/")
}

fn generated_id() -> String {
    "generated-id".to_owned()
}

pub(super) fn strip_bedrock(model: &str, _: Option<&str>, _: Option<&str>) -> Option<String> {
    model.strip_prefix("bedrock/").map(str::to_owned)
}

pub(super) fn cannot_strip(_: &str, _: Option<&str>, _: Option<&str>) -> Option<String> {
    None
}

struct NoModelInfo;

impl ModelInfoLookup for NoModelInfo {
    fn get_model_info(
        &self,
        _model: &str,
        _model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        ready(None)
    }

    fn clear_caches(&self) {}
}

struct NoFetch;

#[derive(Debug, thiserror::Error)]
#[error("these tests never fetch a cost map")]
struct NoFetchError;

impl CostMapFetcher for NoFetch {
    type Error = NoFetchError;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, NoFetchError>> + Send {
        ready(Err(NoFetchError))
    }
}

#[derive(Clone, Copy)]
pub(super) enum Activity {
    Active,
    Inactive,
}

// What the environment and access-window hooks answer, and the PTU and ignore settings of the run.
#[derive(Clone, Copy)]
pub(super) struct Options {
    pub(super) ptu: bool,
    pub(super) ignore: bool,
    pub(super) activity: Activity,
    pub(super) access_error: Option<&'static str>,
    pub(super) duplicates: &'static [&'static str],
}

pub(super) const DEFAULT: Options = Options {
    ptu: false,
    ignore: false,
    activity: Activity::Active,
    access_error: None,
    duplicates: &[],
};

impl ConstructionPolicy for Options {
    fn check_access_windows(
        &self,
        _model_info: &JsonObject,
        _model_name: &str,
    ) -> Result<(), InvalidDeployment> {
        self.access_error.map_or(Ok(()), |message| {
            Err(InvalidDeployment::Rejected {
                message: message.to_owned(),
            })
        })
    }

    fn warn_credential_mismatch(&self, model_name: &str, _litellm_params: &JsonObject) {
        tracing::warn!("credential check ran for {model_name}");
    }

    fn is_active_for_environment(
        &self,
        _model_info: &JsonObject,
    ) -> Result<bool, InvalidDeployment> {
        Ok(matches!(self.activity, Activity::Active))
    }
}

#[derive(Deserialize)]
struct Entry {
    model_name: String,
    litellm_params: JsonObject,
    #[serde(default)]
    model_info: JsonObject,
    #[serde(default)]
    extra: JsonObject,
    declared_id: Option<String>,
}

pub(super) enum Scenario {
    Id {
        group: &'static str,
        params: &'static str,
    },
    Create(&'static str, Options),
    Registered(&'static str),
    Field {
        entry: &'static str,
        options: Options,
        section: &'static str,
        key: &'static str,
    },
    Endpoint {
        path: Option<&'static str>,
        strip: StripProvider,
    },
    Vertex {
        params: &'static str,
        credentials: &'static str,
        provider: &'static str,
    },
    Ignore {
        failure: Failure,
        ignoring: bool,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Failure {
    Unresolved,
    Missing,
}

type Observed = Result<Json, String>;

pub(super) fn expected(answer: Result<&str, &str>) -> Observed {
    match answer {
        Ok(text) => Ok(json(text)),
        Err(message) => Err(message.to_owned()),
    }
}

async fn create(text: &str, options: Options) -> (Result<Option<Json>, String>, ModelRegistry) {
    let entry: Entry = serde_json::from_str(text).unwrap();
    let input = DeploymentInput {
        model_name: entry.model_name,
        litellm_params: entry.litellm_params,
        model_info: entry.model_info,
        extra: entry.extra,
        declared_id: entry.declared_id,
        duplicate_ids: options
            .duplicates
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<BTreeSet<_>>(),
    };
    let mut registry = ModelRegistry::default();
    let generalizations = CatalogGeneralizations::default();
    let environment = RegistrationEnvironment {
        lookup: &NoModelInfo,
        fetcher: &NoFetch,
        generalizations: &generalizations,
        bedrock_base_model: str::to_owned,
    };
    let pricing = PricingInputs {
        ptu_cost_attribution: options.ptu,
        is_strategy_router: is_auto_router,
    };
    let mut run = Construction {
        registry: &mut registry,
        environment: &environment,
        pricing: &pricing,
        policy: &options,
        new_id: generated_id,
        ignore_invalid_deployments: options.ignore,
    };
    let created = create_deployment(&mut run, &input).await;
    let dumped = created
        .map(|made| made.deployment.map(|spec| Json::Object(spec.to_json())))
        .map_err(|failure| failure.to_string());
    (dumped, registry)
}

fn vertex(params: &str, credentials: &str, provider: &str) -> Observed {
    let given: JsonObject = serde_json::from_str(params).map_err(|failure| failure.to_string())?;
    let named: JsonObject =
        serde_json::from_str(credentials).map_err(|failure| failure.to_string())?;
    let found =
        vertex_pass_through(&given, &named, provider).map_err(|failure| failure.to_string())?;
    Ok(found.map_or(Json::Null, |chosen| {
        Json::Object(JsonObject::from([
            ("project".to_owned(), Json::Text(chosen.project)),
            ("location".to_owned(), Json::Text(chosen.location)),
            (
                "credentials".to_owned(),
                chosen.credentials.unwrap_or(Json::Null),
            ),
        ]))
    }))
}

fn ignore_case(failure: Failure, ignoring: bool) -> Observed {
    let error = match failure {
        Failure::Unresolved => {
            DeploymentError::<NoFetchError>::Invalid(InvalidDeployment::ProviderUnresolved {
                message: "LLM Provider NOT provided".to_owned(),
            })
        }
        Failure::Missing => DeploymentError::Invalid(InvalidDeployment::MissingModel),
    };
    ignore_or_raise(ignoring, error)
        .map(Json::Bool)
        .map_err(|raised| raised.to_string())
}

fn endpoint(endpoint: Option<&str>, strip: StripProvider) -> Json {
    let call = PassThroughCall {
        endpoint,
        custom_llm_provider: None,
        api_base: None,
        model: "special",
        model_name: "bedrock/us.claude-v1:0",
    };
    endpoint_for_deployment(&call, strip).map_or(Json::Null, Json::Text)
}

pub(super) async fn observe(scenario: Scenario) -> Observed {
    match scenario {
        Scenario::Id { group, params } => {
            let parsed: JsonObject = serde_json::from_str(params).unwrap();
            Ok(Json::Text(generate_model_id(group, &parsed)))
        }
        Scenario::Create(text, options) => Ok(create(text, options).await.0?.unwrap_or(Json::Null)),
        Scenario::Registered(text) => {
            let (created, registry) = create(text, DEFAULT).await;
            created?;
            Ok(Json::Object(registry.model_cost))
        }
        Scenario::Field {
            entry,
            options,
            section,
            key,
        } => {
            let Some(Json::Object(dumped)) = create(entry, options).await.0? else {
                return Ok(Json::Null);
            };
            let Some(Json::Object(fields)) = dumped.get(section) else {
                return Ok(Json::Null);
            };
            Ok(fields.get(key).cloned().unwrap_or(Json::Null))
        }
        Scenario::Endpoint { path, strip } => Ok(endpoint(path, strip)),
        Scenario::Vertex {
            params,
            credentials,
            provider,
        } => vertex(params, credentials, provider),
        Scenario::Ignore { failure, ignoring } => ignore_case(failure, ignoring),
    }
}

#[derive(Clone)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
