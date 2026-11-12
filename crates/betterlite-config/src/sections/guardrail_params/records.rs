use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::opaque::OpaqueParams;
use super::params::LitellmParams;
use crate::sections::guardrail_identity::enums::Role;

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct LakeraCategoryThresholds {
    pub prompt_injection: Option<f64>,
    pub jailbreak: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Guardrail {
    pub guardrail_id: Option<String>,
    pub guardrail_name: String,
    pub litellm_params: LitellmParams,
    pub guardrail_info: Option<OpaqueParams>,
    pub policy_template: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailEventHooks {
    PreCall,
    PostCall,
    DuringCall,
    LoggingOnly,
    PreMcpCall,
    DuringMcpCall,
    PostMcpCall,
    RealtimeInputTranscription,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DynamicGuardrailParams {
    pub extra_body: OpaqueParams,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardrailDefinitionLocation {
    Db,
    #[default]
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PiiEntityCategoryMap {
    pub category: String,
    pub entities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GuardrailItemSpec {
    pub callbacks: Vec<String>,
    pub default_on: Option<bool>,
    pub logging_only: Option<bool>,
    pub enabled_roles: Option<Vec<Role>>,
    pub callback_args: Option<BTreeMap<String, OpaqueParams>>,
}
