use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::auth_limit_inputs::batch::BatchEnqueuedTokenLimitField;
use betterlite_auth::auth_limit_inputs::deployment::{DeploymentLimitValue, DeploymentLimits};
use betterlite_auth::auth_limit_inputs::error::ManagementEntity;
use betterlite_auth::auth_limit_inputs::estimates::OutputTokenEstimateField;
use betterlite_auth::auth_limit_inputs::metadata_limits::MetadataAccessor;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    KeyModelLimits {
        auth: BTreeMap<String, JsonData>,
        #[serde(default)]
        model_name: Option<String>,
        #[serde(default)]
        deployments: Option<Vec<DeploymentInput>>,
    },
    EstimatedOutputTokens {
        auth: BTreeMap<String, JsonData>,
        #[serde(default)]
        model_name: Option<String>,
    },
    EnforceOutputEstimates {
        request: OutputEstimateRequestInput,
        #[serde(default)]
        existing_metadata: Option<BTreeMap<String, JsonData>>,
        caller: BTreeMap<String, JsonData>,
        entity: EntityInput,
    },
    EnforceBatchLimit {
        request: BatchRequestInput,
        #[serde(default)]
        existing_metadata: Option<BTreeMap<String, JsonData>>,
        caller: BTreeMap<String, JsonData>,
        entity: EntityInput,
    },
    McpRpmLimit {
        auth: BTreeMap<String, JsonData>,
        team_only: bool,
    },
    TagRpmLimit {
        auth: BTreeMap<String, JsonData>,
    },
    TenantFields {
        auth: BTreeMap<String, JsonData>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeploymentInput {
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) default_api_key_rpm_limit: Option<Json>,
    #[serde(default)]
    pub(crate) default_api_key_tpm_limit: Option<Json>,
}

impl DeploymentInput {
    pub(crate) fn limits(&self) -> DeploymentLimits {
        DeploymentLimits {
            default_api_key_rpm_limit: self
                .default_api_key_rpm_limit
                .as_ref()
                .map(DeploymentLimitValue::from),
            default_api_key_tpm_limit: self
                .default_api_key_tpm_limit
                .as_ref()
                .map(DeploymentLimitValue::from),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OutputEstimateRequestInput {
    #[serde(default)]
    pub(crate) metadata: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(crate) model_fields_set: BTreeSet<OutputTokenEstimateField>,
    #[serde(default)]
    pub(crate) default_estimated_output_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) default_estimated_output_tokens_per_model: Option<BTreeMap<String, i64>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BatchRequestInput {
    #[serde(default)]
    pub(crate) metadata: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(crate) model_fields_set: BTreeSet<BatchEnqueuedTokenLimitField>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntityInput {
    Key,
    Team,
}

impl From<EntityInput> for ManagementEntity {
    fn from(value: EntityInput) -> Self {
        match value {
            EntityInput::Key => Self::Key,
            EntityInput::Team => Self::Team,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessorInput {
    TeamMetadata,
    OrganizationMetadata,
    ProjectMetadata,
}

impl From<AccessorInput> for MetadataAccessor {
    fn from(value: AccessorInput) -> Self {
        match value {
            AccessorInput::TeamMetadata => Self::Team,
            AccessorInput::OrganizationMetadata => Self::Organization,
            AccessorInput::ProjectMetadata => Self::Project,
        }
    }
}
