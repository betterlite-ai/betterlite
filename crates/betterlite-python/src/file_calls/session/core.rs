use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigError;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::azure_endpoints;
use betterlite_config::sections::resource_sdk_settings::{Raw, RawSettings, Settings};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_read::target::{FileListing, ReadAnswer, ReadOperation};
use betterlite_core::api::file_provider_read::{ReadContext, ReadEntry};
use betterlite_core::api::file_provider_write::service::FileWrite;
use betterlite_core::api::file_provider_write::{DeleteInput, UploadInput};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::FilesApi;
use chrono::Utc;
use pyo3::PyErr;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::file_calls::convert::{ConvertError, ListCall, RetrieveCall};
use crate::file_calls::dispatch::FileCallDispatch;
use crate::file_calls::error::{read_failure, write_failure};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SessionWire {
    environment: Option<BTreeMap<String, String>>,
    settings: RawSettings,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error("the file call settings are not valid JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("the proxy environment could not be read: {0}")]
    Proxy(#[source] EnvError),
    #[error("the outbound http client could not be built: {0}")]
    Client(#[source] HttpError),
    #[error("the file call settings could not be read: {0}")]
    Settings(#[source] ConfigError),
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

pub(crate) struct FileCallCore {
    client: ProxiedClient,
    settings: Settings,
    environment: BTreeMap<String, String>,
    endpoints: azure_endpoints::Settings,
    catalog: Arc<ModelCatalog>,
    known_models: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    registry: JSONProviderRegistry,
    policies: ProviderPolicies,
    globals: GlobalCredentials,
}

impl FileCallCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = serde_json::from_str(text).map_err(SessionError::Json)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let settings = Raw {
            settings: wire.settings,
        }
        .convert(&environment)
        .map_err(SessionError::Settings)?;
        let endpoints = azure_endpoints::Raw::default()
            .convert(&environment)
            .map_err(SessionError::Settings)?;
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(SessionError::Proxy)?;
        let client = ProxiedClient::outbound(&Arc::new(policy)).map_err(SessionError::Client)?;
        let globals = GlobalCredentials {
            api_key: settings.api_key.clone(),
            api_base: settings.api_base.clone(),
            xai_key: None,
            inception_key: None,
            lemonade_key: None,
        };
        Ok(Self {
            client,
            settings,
            environment,
            endpoints,
            catalog: BundledCatalog::default().get(),
            known_models: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals,
        })
    }

    fn dispatch(&self) -> FileCallDispatch {
        FileCallDispatch {
            openai: FilesApi {
                transport: self.client.clone(),
                boundaries: Uuid::new_v4,
            },
            client: self.client.clone(),
            environment: self.environment.clone(),
            endpoints: self.endpoints.clone(),
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.known_models,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: self.policies.clone(),
            globals: self.globals.clone(),
        }
    }

    pub(crate) async fn create(
        &self,
        input: &UploadInput<(), ()>,
        mode: Mode,
    ) -> Result<String, PyErr> {
        let resolution = self.resolution();
        let service = FileWrite {
            providers: self.dispatch(),
            settings: ResourceSdkSettings::from(&self.settings),
            resolution: &resolution,
            clock: || Utc::now().fixed_offset(),
            call_ids: Uuid::new_v4,
        };
        let object = service
            .create_file(mode, input)
            .await
            .map_err(|error| PyErr::from(write_failure(&error)))?;
        answer_text(&object)
    }

    pub(crate) async fn delete(
        &self,
        input: &DeleteInput<(), ()>,
        mode: Mode,
    ) -> Result<String, PyErr> {
        let resolution = self.resolution();
        let service = FileWrite {
            providers: self.dispatch(),
            settings: ResourceSdkSettings::from(&self.settings),
            resolution: &resolution,
            clock: || Utc::now().fixed_offset(),
            call_ids: Uuid::new_v4,
        };
        let deleted = service
            .delete_file(mode, input)
            .await
            .map_err(|error| PyErr::from(write_failure(&error)))?;
        answer_text(&deleted)
    }

    pub(crate) async fn retrieve(
        &self,
        call: &RetrieveCall,
        entry: ReadEntry,
    ) -> Result<String, PyErr> {
        let service = FileProviderRead {
            dispatch: self.dispatch(),
            settings: self.settings.clone(),
            clock: || Utc::now().fixed_offset(),
            call_ids: Uuid::new_v4,
        };
        let context: ReadContext<'_, ()> = ReadContext {
            trusted: Some(&call.trusted),
            client: None,
            logger: None,
        };
        let answer = service
            .run(
                entry,
                ReadOperation::Retrieve {
                    file_id: &call.file_id,
                },
                &call.kwargs,
                context,
            )
            .await
            .map_err(|error| PyErr::from(read_failure(&error)))?;
        match answer {
            ReadAnswer::File(object) => answer_text(&object),
            ReadAnswer::Listing(_) => Err(PyRuntimeError::new_err(
                "file_retrieve answered with a file list",
            )),
        }
    }

    pub(crate) async fn list(&self, call: &ListCall, entry: ReadEntry) -> Result<String, PyErr> {
        let service = FileProviderRead {
            dispatch: self.dispatch(),
            settings: self.settings.clone(),
            clock: || Utc::now().fixed_offset(),
            call_ids: Uuid::new_v4,
        };
        let context: ReadContext<'_, ()> = ReadContext {
            trusted: Some(&call.trusted),
            client: None,
            logger: None,
        };
        let answer = service
            .run(
                entry,
                ReadOperation::List {
                    purpose: call.purpose.as_deref(),
                },
                &call.kwargs,
                context,
            )
            .await
            .map_err(|error| PyErr::from(read_failure(&error)))?;
        match answer {
            ReadAnswer::Listing(FileListing::Page(page)) => answer_text(&page),
            ReadAnswer::File(_) | ReadAnswer::Listing(FileListing::Files(_)) => Err(
                PyRuntimeError::new_err("file_list answered with another reply kind"),
            ),
        }
    }
}

fn answer_text<T: Serialize>(value: &T) -> Result<String, PyErr> {
    serde_json::to_string(value).map_err(|source| PyErr::from(ConvertError::Answer(source)))
}
