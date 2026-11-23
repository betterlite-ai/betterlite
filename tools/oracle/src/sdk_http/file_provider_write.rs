use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::file_provider_write::service::FileWrite;
use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use chrono::Utc;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::rust::double_option;
use uuid::Uuid;

use self::input::run_step;
use self::providers::OracleProviders;
use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use super::vector_query_embedding::outbound_client;

mod input;
mod outcome;
mod providers;

// one case drives several create_file, acreate_file, file_delete and afile_delete calls against one mock
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub steps: Vec<Step>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    CreateFile(UploadStep),
    #[serde(rename = "acreate_file")]
    ACreateFile(UploadStep),
    FileDelete(DeleteStep),
    #[serde(rename = "afile_delete")]
    AFileDelete(DeleteStep),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Purpose {
    Assistants,
    Batch,
    FineTune,
    Messages,
}
// the (filename, content, content_type) tuple the caller uploads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileArg {
    pub filename: String,
    pub content_type: String,
    pub content: String,
}
// FileExpiresAfter's anchor is always created_at
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpiresArg {
    pub anchor: Anchor,
    pub seconds: i64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    CreatedAt,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadStep {
    pub file: FileArg,
    pub purpose: Purpose,
    #[serde(default)]
    pub expires_after: Option<ExpiresArg>,
    // absent takes the function's default and null is sent as None
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub custom_llm_provider: Option<Option<String>>,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub params: Params,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteStep {
    pub file_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub params: Params,
}
// the kwargs both calls read through GenericLiteLLMParams, beside request_timeout
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<ReadTimeout>,
    pub request_timeout: Option<ReadTimeoutValue>,
    pub max_retries: Option<i64>,
    pub passthrough: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}
// value is the file object an upload returns or the FileDeleted record a delete returns
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bad_request: Option<Rejection>,
}
// the synthesized httpx.Response and httpx.Request a LiteLLM BadRequestError carries
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejection {
    pub model: String,
    pub llm_provider: String,
    pub body: String,
    pub method: String,
    pub url: String,
}
impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.steps.is_empty() {
            return Err(SdkHttpError::Invalid(
                "a file write call needs at least one step".to_owned(),
            ));
        }
        if self.steps.len() > limits.max_requests {
            return Err(SdkHttpError::Invalid(format!(
                "a file write call has {} steps, past the limit of {}",
                self.steps.len(),
                limits.max_requests
            )));
        }
        Ok(())
    }

    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        let loaded = ConfigLoader {
            lookup: env.clone(),
        }
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
        let catalog = ModelCatalog::default();
        let names = BTreeSet::new();
        let resolution = ResolutionContext {
            catalog: &catalog,
            known_model_names: &names,
            custom_providers: &names,
            environment: env,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let write = FileWrite {
            providers: OracleProviders::new(outbound_client(env)?),
            settings: ResourceSdkSettings::from(&loaded.sections.resource_sdk_settings),
            resolution: &resolution,
            clock: || Utc::now().fixed_offset(),
            call_ids: Uuid::new_v4,
        };
        let mut steps = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            steps.push(run_step(&write, step, api_base).await?);
        }
        Ok(SdkResult::FileProviderWrite {
            value: Box::new(Outcome { steps }),
        })
    }
}
