use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;

use super::base::BaseLitellmParams;
use super::opaque::OpaqueParams;
use super::records::{GuardrailDefinitionLocation, PiiEntityCategoryMap};
use crate::sections::guardrail_identity::enums::PiiEntityType;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailInfoResponse {
    pub guardrail_id: Option<String>,
    pub guardrail_name: String,
    pub litellm_params: Option<BaseLitellmParams>,
    pub guardrail_info: Option<OpaqueParams>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub guardrail_definition_location: GuardrailDefinitionLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListGuardrailsResponse {
    pub guardrails: Vec<GuardrailInfoResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardrailUiAddSettings {
    pub supported_entities: Vec<String>,
    pub supported_actions: Vec<String>,
    pub supported_modes: Vec<String>,
    pub supported_modes_by_provider: BTreeMap<String, Vec<String>>,
    pub pii_entity_categories: Vec<PiiEntityCategoryMap>,
    pub content_filter_settings: Option<OpaqueParams>,
    #[serde(default)]
    pub decision_model_providers: Vec<String>,
}

#[serde_inline_default]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyGuardrailRequest {
    pub guardrail_name: String,
    pub text: String,
    pub language: Option<String>,
    pub entities: Option<Vec<PiiEntityType>>,
    #[serde_inline_default("request".to_owned())]
    pub input_type: String,
    pub messages: Option<Vec<OpaqueParams>>,
    pub metadata: Option<OpaqueParams>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyGuardrailResponse {
    pub response_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchGuardrailRequest {
    pub guardrail_name: Option<String>,
    pub litellm_params: Option<BaseLitellmParams>,
    pub guardrail_info: Option<OpaqueParams>,
}
