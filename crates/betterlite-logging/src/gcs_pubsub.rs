use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_env::EnvError;
use betterlite_models::decisions::Json;
use http::header::InvalidHeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use url::ParseError;

use crate::batch::logger::{BatchQueue, BatchSettings};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::gcs_mock::auth::VertexAuth;
use crate::http_analytics_loggers::delivery::Delivery;
use crate::registry::catalog::gcs_pubsub::ProjectionError;

pub const CALLBACK_NAME: &str = "gcs_pubsub";
pub const PROJECT_VARIABLE: &str = "GCS_PUBSUB_PROJECT_ID";
pub const TOPIC_VARIABLE: &str = "GCS_PUBSUB_TOPIC_ID";
pub const CREDENTIALS_VARIABLE: &str = "GCS_PATH_SERVICE_ACCOUNT";
pub const USE_V1_VARIABLE: &str = "GCS_PUB_SUB_USE_V1";

#[derive(Debug, thiserror::Error)]
pub enum PubSubError<Auth, D>
where
    Auth: VertexAuth,
    D: Delivery,
{
    #[error("GCS_PUBSUB_PROJECT_ID is not set and no project id was given")]
    MissingProjectId,
    #[error("GCS_PUBSUB_TOPIC_ID is not set and no topic id was given")]
    MissingTopicId,
    #[error("the {name} setting is not valid")]
    Variable {
        name: &'static str,
        #[source]
        source: EnvError,
    },
    #[error("the Pub/Sub request headers could not be built")]
    Headers {
        #[source]
        source: Auth::Error,
    },
    #[error("a Pub/Sub request header is not valid")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the {record} could not be written as JSON")]
    Json {
        record: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the spend log record could not be built: {source}")]
    Projection {
        #[from]
        source: ProjectionError,
    },
    #[error("the Pub/Sub publish URL is not valid")]
    Url {
        #[source]
        source: ParseError,
    },
    #[error("the Pub/Sub publish failed")]
    Transport {
        #[source]
        source: D::Error,
    },
    #[error("Pub/Sub refused the message with status {status}")]
    Status { status: u16 },
}

#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub project_id: Option<String>,
    pub topic_id: Option<String>,
    pub credentials: Option<SecretString>,
    pub use_v1: Option<bool>,
}

pub(crate) struct Settings {
    pub(crate) project_id: String,
    pub(crate) topic_id: String,
    pub(crate) credentials: Option<SecretString>,
    use_v1: bool,
}

pub struct GcsPubSubDeps<Auth, D>
where
    Auth: VertexAuth,
    D: Delivery,
{
    pub auth: Auth,
    pub delivery: D,
    pub project: fn(&LoggerEvent) -> Result<Json, ProjectionError>,
    pub batch: BatchSettings,
}

pub struct GcsPubSubLogger<Auth, D>
where
    Auth: VertexAuth,
    D: Delivery,
{
    pub(crate) settings: Settings,
    pub(crate) queue: BatchQueue<Json>,
    pub(crate) auth: Auth,
    pub(crate) delivery: D,
    project: fn(&LoggerEvent) -> Result<Json, ProjectionError>,
}

impl<Auth, D> GcsPubSubLogger<Auth, D>
where
    Auth: VertexAuth,
    D: Delivery,
{
    pub fn new(
        environment: &dyn EnvironmentLookup,
        overrides: Overrides,
        deps: GcsPubSubDeps<Auth, D>,
    ) -> Result<Self, PubSubError<Auth, D>> {
        let project_id = overrides
            .project_id
            .filter(|id| !id.is_empty())
            .or_else(|| environment.lookup(PROJECT_VARIABLE))
            .filter(|id| !id.is_empty())
            .ok_or(PubSubError::MissingProjectId)?;
        let topic_id = overrides
            .topic_id
            .filter(|id| !id.is_empty())
            .or_else(|| environment.lookup(TOPIC_VARIABLE))
            .filter(|id| !id.is_empty())
            .ok_or(PubSubError::MissingTopicId)?;
        let credentials = overrides
            .credentials
            .filter(|secret| !secret.expose_secret().is_empty())
            .or_else(|| {
                environment
                    .lookup(CREDENTIALS_VARIABLE)
                    .map(SecretString::from)
            })
            .filter(|secret| !secret.expose_secret().is_empty());
        let configured_v1 = environment
            .lookup(USE_V1_VARIABLE)
            .map(|text| betterlite_env::flag(USE_V1_VARIABLE, &text))
            .transpose()
            .map_err(|source| PubSubError::Variable {
                name: USE_V1_VARIABLE,
                source,
            })?
            .flatten();
        let use_v1 = overrides.use_v1.or(configured_v1).unwrap_or(false);
        Ok(Self {
            settings: Settings {
                project_id,
                topic_id,
                credentials,
                use_v1,
            },
            queue: BatchQueue::new(deps.batch),
            auth: deps.auth,
            delivery: deps.delivery,
            project: deps.project,
        })
    }
}

impl<Auth, D> GcsPubSubLogger<Auth, D>
where
    Auth: VertexAuth + 'static,
    D: Delivery + 'static,
{
    async fn record(&self, event: &LoggerEvent) {
        match self.select(event) {
            Ok(message) => {
                let queued = self.queue.enqueue(message);
                if queued.depth >= self.queue.settings.batch_size {
                    self.send_batch().await;
                }
            }
            Err(error) => tracing::error!(%error, "PubSub Layer Error"),
        }
    }

    fn select(&self, event: &LoggerEvent) -> Result<Json, PubSubError<Auth, D>> {
        if self.settings.use_v1 {
            return (self.project)(event).map_err(|source| PubSubError::Projection { source });
        }
        serde_json::to_value(event.payload.as_ref())
            .and_then(serde_json::from_value::<Json>)
            .map_err(|source| PubSubError::Json {
                record: "standard logging payload",
                source,
            })
    }
}

#[async_trait]
impl<Auth, D> CallbackLogger for GcsPubSubLogger<Auth, D>
where
    Auth: VertexAuth + 'static,
    D: Delivery + 'static,
{
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    // LiteLLM's Pub/Sub logger inherits CustomBatchLogger's no-op failure handler, so a failure event publishes nothing
    async fn async_log_failure_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record(&event).await;
        Ok(())
    }
}

mod tests;
