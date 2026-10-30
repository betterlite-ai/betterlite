use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::{
    AutoRotateFlag, DefaultFilledList, ExposedSecret, Payload, PydanticBool, ZeroRotationCount,
    integer_option, serialize_as_float,
};
use crate::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expiry {
    Text(String),
    DateTime(DateTime<FixedOffset>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLLMVerificationToken {
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub token: Option<SecretString>,
    #[serde(default)]
    pub key_name: Option<String>,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(
        default,
        serialize_with = "serialize_as_float",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub spend: Decimal,
    #[serde(
        default,
        serialize_with = "serialize_as_float",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub total_spend: Decimal,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default)]
    pub expires: Option<Expiry>,
    #[serde(default)]
    pub models: Vec<JsonData>,
    #[serde(default)]
    pub aliases: Payload,
    #[serde(default)]
    pub config: Payload,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default, deserialize_with = "integer_option")]
    pub max_parallel_requests: Option<i64>,
    #[serde(default)]
    pub metadata: Payload,
    #[serde(default, deserialize_with = "integer_option")]
    pub tpm_limit: Option<i64>,
    #[serde(default, deserialize_with = "integer_option")]
    pub rpm_limit: Option<i64>,
    #[serde(default, deserialize_with = "integer_option")]
    pub tpd_limit: Option<i64>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
    #[serde(default)]
    pub allowed_cache_controls: DefaultFilledList<JsonData>,
    #[serde(default)]
    pub allowed_routes: DefaultFilledList<JsonData>,
    #[serde(default)]
    pub key_type: Option<String>,
    #[serde(default)]
    pub permissions: Payload,
    #[serde(default)]
    pub model_spend: Payload,
    #[serde(default)]
    pub model_max_budget: Payload,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::MapPreventDuplicates<serde_with::Same, serde_with::Same>>::deserialize"
    )]
    pub budget_fallbacks: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub soft_budget_cooldown: PydanticBool,
    #[serde(default)]
    pub blocked: Option<PydanticBool>,
    #[serde(default)]
    pub litellm_budget_table: Option<Payload>,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub settings_updated_at: Option<Timestamp>,
    #[serde(default)]
    pub last_active: Option<Timestamp>,
    #[serde(default)]
    pub object_permission_id: Option<String>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    #[serde(default)]
    pub access_group_ids: Option<Vec<String>>,
    #[serde(default)]
    pub rotation_count: ZeroRotationCount,
    #[serde(default)]
    pub auto_rotate: AutoRotateFlag,
    #[serde(default)]
    pub rotation_interval: Option<String>,
    #[serde(default)]
    pub last_rotation_at: Option<Timestamp>,
    #[serde(default)]
    pub key_rotation_at: Option<Timestamp>,
    #[serde(default)]
    pub router_settings: Option<Payload>,
    #[serde(default)]
    pub budget_limits: Option<Vec<Payload>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedVerificationToken {
    #[serde(flatten)]
    pub token: LiteLLMVerificationToken,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub deleted_at: Option<Timestamp>,
    #[serde(default)]
    pub deleted_by: Option<String>,
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub deleted_by_api_key: Option<SecretString>,
    #[serde(default)]
    pub litellm_changed_by: Option<String>,
}
