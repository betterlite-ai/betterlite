use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroU64;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;
use strum::AsRefStr;

use super::content_filter::ContentFilterConfigModel;
use super::decode::decode_fields;
use super::error::GuardrailParamsError;
use super::keys::BASE_KEYS;
use super::opaque::OpaqueParams;
use super::records::LakeraCategoryThresholds;
use crate::sections::guardrail_identity::item::GuardrailValue;
use crate::sections::guardrail_translation::ExposedSecret;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreachableFallback {
    #[default]
    FailClosed,
    FailOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OnViolation {
    Warn,
    EndSession,
    Block,
    Alert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnSensitiveData {
    Block,
    Route,
}

// clippy allows two bools per struct, so the other four sit in these two
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ScanFlags {
    #[serde(default, rename = "experimental_use_latest_role_message_only")]
    pub latest_role_message_only: bool,
    #[serde(default)]
    pub only_scan_new_messages: bool,
}

#[serde_inline_default]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HandlingFlags {
    #[serde(default)]
    pub skip_unscannable_attachments: bool,
    #[serde_inline_default(true)]
    pub sticky_session_routing: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SentKeys {
    pub(super) sent: BTreeSet<String>,
    pub(super) null: BTreeSet<String>,
}

#[serde_inline_default]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseParamFields {
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    #[serde(rename = "skip_system_message_in_guardrail")]
    pub skip_system_message: Option<bool>,
    pub skip_tool_message_in_guardrail: Option<bool>,
    pub scan_only_tool_results: Option<bool>,
    pub inspect_embeddings: Option<bool>,
    pub category_thresholds: Option<LakeraCategoryThresholds>,
    pub detect_secrets_config: Option<OpaqueParams>,
    pub guard_name: Option<String>,
    pub default_on: Option<bool>,
    pub mask_request_content: Option<bool>,
    pub mask_response_content: Option<bool>,
    pub pangea_input_recipe: Option<String>,
    pub pangea_output_recipe: Option<String>,
    pub model: Option<String>,
    pub violation_message_template: Option<String>,
    pub end_session_after_n_fails: Option<i64>,
    pub on_violation: Option<OnViolation>,
    pub realtime_violation_message: Option<String>,
    pub template_id: Option<String>,
    pub location: Option<String>,
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub credentials: Option<SecretString>,
    pub api_endpoint: Option<String>,
    #[serde_inline_default(true)]
    pub fail_on_error: bool,
    #[serde_inline_default(true)]
    pub sanitize_error_detail: bool,
    #[serde(rename = "additional_provider_specific_params")]
    pub provider_specific_params: Option<OpaqueParams>,
    #[serde(default)]
    pub unreachable_fallback: UnreachableFallback,
    pub max_input_chars: Option<NonZeroU64>,
    pub max_concurrent_decision_calls: Option<NonZeroU64>,
    pub extra_headers: Option<Vec<String>>,
    pub custom_code: Option<String>,
    pub timeout: Option<f64>,
    pub on_sensitive_data: Option<OnSensitiveData>,
    pub sensitive_data_route_to_model: Option<String>,
    pub run_in_parallel: Option<bool>,
    pub scan_raw_request: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "OpaqueParams")]
pub struct BaseLitellmParams {
    #[serde(flatten)]
    pub fields: BaseParamFields,
    #[serde(flatten)]
    pub scan_flags: ScanFlags,
    #[serde(flatten)]
    pub handling_flags: HandlingFlags,
    #[serde(flatten)]
    pub content_filter: ContentFilterConfigModel,
    #[serde(flatten)]
    pub extra: OpaqueParams,
    #[serde(skip)]
    pub keys: SentKeys,
}

impl fmt::Debug for BaseLitellmParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BaseLitellmParams")
            .field("extra", &self.extra)
            .finish_non_exhaustive()
    }
}

impl TryFrom<OpaqueParams> for BaseLitellmParams {
    type Error = GuardrailParamsError;

    fn try_from(params: OpaqueParams) -> Result<Self, Self::Error> {
        Self::from_map(params.0)
    }
}

impl BaseLitellmParams {
    pub fn from_map(map: BTreeMap<String, GuardrailValue>) -> Result<Self, GuardrailParamsError> {
        let keys = SentKeys {
            sent: map.keys().cloned().collect(),
            null: map
                .iter()
                .filter(|(_key, value)| **value == GuardrailValue::Null)
                .map(|(key, _value)| key.clone())
                .collect(),
        };
        let (known, extra): (BTreeMap<_, _>, BTreeMap<_, _>) = map
            .into_iter()
            .partition(|(key, _value)| BASE_KEYS.contains(&key.as_str()));
        let mut declared: BTreeMap<_, _> = known
            .into_iter()
            .filter(|(_key, value)| *value != GuardrailValue::Null)
            .collect();
        for key in ["unreachable_fallback", "on_sensitive_data"] {
            if let Some(GuardrailValue::Text(text)) = declared.get_mut(key) {
                *text = text.to_lowercase();
            }
        }
        let bytes = serde_json::to_vec(&declared)
            .map_err(|source| GuardrailParamsError::Encode { source })?;
        let fields: BaseParamFields = decode_fields(&bytes)?;
        let route_missing = fields
            .sensitive_data_route_to_model
            .as_deref()
            .is_none_or(str::is_empty);
        if fields.on_sensitive_data == Some(OnSensitiveData::Route) && route_missing {
            return Err(GuardrailParamsError::RouteModelMissing);
        }
        Ok(Self {
            fields,
            scan_flags: decode_fields(&bytes)?,
            handling_flags: decode_fields(&bytes)?,
            content_filter: decode_fields(&bytes)?,
            extra: OpaqueParams(extra),
            keys,
        })
    }

    pub fn sent_fields(&self) -> Result<BTreeMap<String, GuardrailValue>, GuardrailParamsError> {
        let bytes =
            serde_json::to_vec(self).map_err(|source| GuardrailParamsError::Encode { source })?;
        let mut dump = serde_json::from_slice::<OpaqueParams>(&bytes)
            .map_err(|source| GuardrailParamsError::Encode { source })?
            .0;
        let mut sent: BTreeMap<String, GuardrailValue> = self
            .keys
            .sent
            .iter()
            .filter_map(|key| dump.remove(key).map(|value| (key.clone(), value)))
            .collect();
        for key in &self.keys.null {
            sent.insert(key.clone(), GuardrailValue::Null);
        }
        Ok(sent)
    }
}
