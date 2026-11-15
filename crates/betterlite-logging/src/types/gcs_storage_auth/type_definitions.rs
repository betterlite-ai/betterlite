use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use secrecy::SecretString;

use crate::fields::payload::StandardLoggingPayload;
use crate::gcs_client::gcs_storage_auth::VertexInstance;

pub struct GcsLoggingConfig<Auth> {
    pub bucket_name: String,
    pub vertex_instance: VertexInstance<Auth>,
    pub path_service_account: Option<SecretString>,
}

#[derive(Clone, Debug, Default)]
pub struct GcsLoggingKwargs {
    pub dynamic_params: Option<StandardCallbackDynamicParams>,
}

#[derive(Clone, Debug)]
pub struct GcsLogQueueItem {
    pub payload: StandardLoggingPayload,
    pub kwargs: GcsLoggingKwargs,
    pub response_obj: Option<Json>,
}

#[derive(Clone, Debug)]
pub enum GcsLoggingPayload {
    Text(String),
    Standard(Box<StandardLoggingPayload>),
}

mod tests;
