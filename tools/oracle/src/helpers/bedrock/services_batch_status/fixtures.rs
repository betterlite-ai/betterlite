use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;

pub(crate) type Object = indexmap::IndexMap<String, Json>;

#[derive(Debug, Deserialize)]
pub(crate) struct CredentialInput {
    pub(crate) access_key_id: SecretString,
    pub(crate) secret_access_key: SecretString,
    pub(crate) session_token: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Retrieve {
    pub(crate) batch_id: String,
    pub(crate) litellm_params: Object,
    pub(crate) optional_params: Object,
    pub(crate) now: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Prediction {
    pub(crate) output_prefix: String,
    pub(crate) input_uri: String,
    pub(crate) job_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TimeInput {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(String),
    Text(String),
    Null(()),
    Nonfinite(String),
}
