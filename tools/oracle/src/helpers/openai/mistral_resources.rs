use std::collections::BTreeMap;

use betterlite_config::sections::mistral_resources::Settings;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::mistral_batches::records::MistralBatchJob;
use betterlite_openai::mistral_files::records::{MistralFile, MistralFileDeleted};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

mod batches;
mod files;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Files {
        #[serde(default)]
        env: BTreeMap<String, String>,
        calls: Vec<FilesCall>,
    },
    BatchCreate {
        #[serde(default)]
        env: BTreeMap<String, String>,
        calls: Vec<BatchCreateCall>,
    },
    Batch {
        calls: Vec<BatchCall>,
    },
    Retrieve {
        #[serde(default)]
        env: BTreeMap<String, String>,
        calls: Vec<RetrieveCall>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilesCall {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) now: i64,
    #[serde(default)]
    pub(super) uploads: Vec<UploadCall>,
    #[serde(default)]
    pub(super) list_purposes: Vec<Option<String>>,
    #[serde(default)]
    pub(super) bodies: Vec<MistralFile>,
    #[serde(default)]
    pub(super) deleted: Vec<MistralFileDeleted>,
    #[serde(default)]
    pub(super) file_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadCall {
    pub(super) file: UploadFile,
    pub(super) purpose: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadFile {
    pub(super) filename: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) content: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchCreateCall {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) data: CreateBatchRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchCall {
    pub(super) batch: MistralBatchJob,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveCall {
    pub(super) batch_id: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
}

fn resource_settings(env: &BTreeMap<String, String>) -> Settings {
    Settings {
        api_base: env
            .get("MISTRAL_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned(),
        api_key: env
            .get("MISTRAL_API_KEY")
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.as_str())),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Files { env, calls } => {
                let settings = resource_settings(env);
                let outcomes: Vec<Value> = calls
                    .iter()
                    .map(|call| files::outcome(call, &settings))
                    .collect();
                Ok(json!({"calls": outcomes}))
            }
            Input::BatchCreate { env, calls } => {
                let settings = resource_settings(env);
                let outcomes: Vec<Value> = calls
                    .iter()
                    .map(|call| batches::create(call, &settings))
                    .collect();
                Ok(json!({"calls": outcomes}))
            }
            Input::Batch { calls } => {
                let outcomes: Vec<Value> =
                    calls.iter().map(|call| batches::job(&call.batch)).collect();
                Ok(json!({"calls": outcomes}))
            }
            Input::Retrieve { env, calls } => {
                let settings = resource_settings(env);
                let outcomes: Vec<Value> = calls
                    .iter()
                    .map(|call| batches::retrieve(call, &settings))
                    .collect();
                Ok(json!({"calls": outcomes}))
            }
        }
    }
}
